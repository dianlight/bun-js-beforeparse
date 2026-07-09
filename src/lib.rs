//! bun-js-beforeparse
//!
//! A generic Bun bundler plugin bridge that lets you write `onBeforeParse`
//! transforms in plain TypeScript/JavaScript, without any per-project Rust code.
//!
//! # Architecture
//!
//! Bun's `onBeforeParse` hook is called from multiple native worker threads.
//! JS is single-threaded. The bridge connects them via a rendezvous channel:
//!
//!   Bun worker thread                       JS main thread
//!   ─────────────────                       ─────────────────
//!   bun_js_bridge_dispatch()                TSFN callback fires
//!     creates SyncChannel(0)                  calls user's JS fn (sync or async)
//!     posts { source, path, tx } via TSFN     gets the String result
//!     blocks on rx.recv()                     sender.send(result)
//!     writes result → OnBeforeParseResult
//!
//! Multiple worker threads block independently — each has its own channel pair.

#![deny(clippy::all)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::panic;
use std::sync::{Arc, Mutex, mpsc};

use bun_native_plugin::{define_bun_plugin, OnBeforeParse};

// Export the BUN_PLUGIN_NAME symbol that Bun requires to validate this NAPI module.
// Without this, build.onBeforeParse() throws "must be a Napi module which exports BUN_PLUGIN_NAME".
define_bun_plugin!("bun-js-beforeparse");
use napi::threadsafe_function::{
    ErrorStrategy, ThreadSafeCallContext, ThreadsafeFunction, ThreadsafeFunctionCallMode,
};
use napi::{bindgen_prelude::*, Env, JsFunction, JsUnknown};
use napi_derive::napi;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Payload sent from a Bun worker thread → JS main thread via the TSFN.
/// Note: `tx` is NOT used in the TSFN setup closure — it's moved into the
/// `call_with_return_value` callback that runs on the JS thread after the user's
/// function returns. See dispatch_inner() for the full flow.
struct BridgePayload {
    source: String,
    path: String,
}

// SAFETY: BridgePayload crosses thread boundaries via TSFN — all fields are Send.
unsafe impl Send for BridgePayload {}

/// Holds the ThreadsafeFunction behind a Mutex so release_bridge can call unref(&mut self).
/// Wrapped in Arc so the extern "C" hook can use it from concurrent worker threads.
///
/// Uses ErrorStrategy::CalleeHandled (the default from create_threadsafe_function).
/// IMPORTANT: CalleeHandled prepends a null "error" arg following Node.js convention,
/// so the JS callback receives (null, source, path). The JS wrapper in index.ts handles
/// this by wrapping: (_err, source, path) => userFn(source, path).
pub struct BridgeFn {
    tsfn: Mutex<ThreadsafeFunction<BridgePayload, ErrorStrategy::CalleeHandled>>,
}

// SAFETY: ThreadsafeFunction is Send+Sync per napi-rs.
unsafe impl Send for BridgeFn {}
unsafe impl Sync for BridgeFn {}

// ─── NAPI export: create_bridge ─────────────────────────────────────────────

/// Called from TypeScript's `jsBridge(fn)` to register the user's JS callback.
///
/// Returns `External<Arc<BridgeFn>>` — an opaque value passed as `external` in
/// `build.onBeforeParse(matcher, { napiModule, symbol, external })`.
/// Bun hands this pointer back to `bun_js_bridge_dispatch` on every file.
#[napi]
pub fn create_bridge(
    env: Env,
    callback: JsFunction,
) -> Result<External<Arc<BridgeFn>>> {
    // Build a ThreadsafeFunction<BridgePayload, Fatal>.
    // The closure runs on the JS thread and converts the payload into call arguments.
    // Fatal means any error in the closure terminates via panic (we handle errors below).
    let tsfn: ThreadsafeFunction<BridgePayload, ErrorStrategy::CalleeHandled> = env
        .create_threadsafe_function(
            &callback,
            0, // queue size 0 = unlimited; each call blocks the worker anyway
            |ctx: ThreadSafeCallContext<BridgePayload>| -> Result<Vec<String>> {
                // Build JS args: (source: string, path: string)
                // Return Vec<String> — String implements ToNapiValue in napi v2,
                // and napi-rs will create the JS strings during the actual call.
                Ok(vec![ctx.value.source.clone(), ctx.value.path.clone()])
            },
        )?;

    // Wrap in Arc<BridgeFn> (BridgeFn holds a Mutex<TSFN>) so dispatch can be called
    // from concurrent worker threads without needing exclusive ownership.
    Ok(External::new(Arc::new(BridgeFn { tsfn: Mutex::new(tsfn) })))
}

/// Call this after your build completes to release the TSFN reference, allowing
/// the event loop to exit. Without this, the process hangs waiting for more calls.
///
/// Consumes the External. After this call, do not pass the external to onBeforeParse.
#[napi]
pub fn release_bridge(env: Env, bridge: External<Arc<BridgeFn>>) -> Result<()> {
    let bridge_fn: &Arc<BridgeFn> = &bridge;
    if let Ok(mut tsfn) = bridge_fn.tsfn.lock() {
        let _ = tsfn.unref(&env);
    }
    Ok(())
}

// ─── NAPI export: get_result_from_js ────────────────────────────────────────

/// Called from the JS side of the TSFN after the user's transform completes.
///
/// The JS wrapper (`js/index.ts`) wraps the user callback so that when the
/// transform returns (sync or async), it calls this function to route the
/// result back to the blocked worker thread via the channel stored in the payload.
///
/// This avoids the complexity of routing Promise resolution back through Rust.
/// Instead: the TSFN callback returns void; a separate JS→Rust call delivers
/// the result. The payload's `tx` sender is stored on a global WeakMap keyed
/// by call ID.
///
/// **Implementation note:** The simpler single-round-trip approach below stores
/// the SyncSender inside the BridgePayload which is passed to the TSFN. The JS
/// callback is expected to call `sendBridgeResult(callId, result)` synchronously
/// (or after awaiting the user fn). We implement this with call_with_return_value
/// in the dispatch function instead.
#[allow(dead_code)]
fn _placeholder() {}

// ─── extern "C" hook: bun_js_bridge_dispatch ────────────────────────────────

/// The native `onBeforeParse` symbol that Bun calls for each matched file.
///
/// Registration:
/// ```ts
/// build.onBeforeParse(
///   { filter: /\.[jt]sx$/, namespace: "file" },
///   { napiModule, symbol: "bun_js_bridge_dispatch", external },
/// )
/// ```
///
/// Blocks the calling Bun worker thread while the JS transform runs.
/// On any failure, returns the original source unchanged.
#[no_mangle]
pub extern "C" fn bun_js_bridge_dispatch(
    args: *mut bun_native_plugin::sys::OnBeforeParseArguments,
    result: *mut bun_native_plugin::sys::OnBeforeParseResult,
) {
    // catch_unwind prevents a Rust panic from crashing the Bun runtime.
    let _ = panic::catch_unwind(|| {
        dispatch_inner(args, result);
    });
    // If dispatch_inner panicked, result is untouched → Bun uses original source.
}

fn dispatch_inner(
    args: *mut bun_native_plugin::sys::OnBeforeParseArguments,
    result: *mut bun_native_plugin::sys::OnBeforeParseResult,
) {
    // SAFETY: Bun guarantees args and result are valid for this call's duration.
    let mut handle = match OnBeforeParse::from_raw(args, result) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[bun-js-beforeparse] from_raw error: {e}");
            return;
        }
    };

    // File path — owned String to send across the thread boundary.
    let path = match handle.path() {
        Ok(cow) => cow.into_owned(),
        Err(e) => {
            eprintln!("[bun-js-beforeparse] path error: {e}");
            return;
        }
    };

    // Source code — input_source_code() is lazy; the actual bytes are fetched here.
    // We call into_owned() to get a String (copies once; needed to send across threads).
    let source = match handle.input_source_code() {
        Ok(cow) => cow.into_owned(),
        Err(e) => {
            eprintln!("[bun-js-beforeparse] input_source_code error: {e}");
            return;
        }
    };

    // Preserve Bun's detected loader (jsx, tsx, ts, js, …) for the output.
    let loader = handle.output_loader();

    // Retrieve the Arc<BridgeFn> stored in the `external` field.
    // The closure re-interprets the raw c_void pointer as &Arc<BridgeFn>.
    //
    // SAFETY: The pointer was created by External::new(Arc::new(BridgeFn {...}))
    // in create_bridge(); it is kept alive by the JS External object. No other
    // invocation holds a mutable reference to it.
    // napi v2 External<T> stores data as *mut TaggedObject<T> (with a type tag).
    // We must use External::<Arc<BridgeFn>>::inner_from_raw(ptr) to read through
    // the TaggedObject wrapper correctly. Direct casting segfaults.
    let bridge: &Arc<BridgeFn> = match unsafe {
        handle.external(|ptr: *mut c_void| -> Option<&Arc<BridgeFn>> {
            External::<Arc<BridgeFn>>::inner_from_raw(ptr)
        })
    } {
        Ok(Some(b)) => b,
        Ok(None) => {
            eprintln!("[bun-js-beforeparse] external is null (was jsBridge external passed?)");
            return;
        }
        Err(e) => {
            eprintln!("[bun-js-beforeparse] external() error: {e}");
            return;
        }
    };

    // Rendezvous channel (capacity 0): sender.send() blocks until receiver.recv() picks up.
    // The tx is moved into the call_with_return_value callback (runs on JS thread after
    // the user's function returns). rx blocks this worker thread until the result arrives.
    let (tx, rx) = mpsc::sync_channel::<String>(0);

    // call_with_return_value<JsUnknown, _>:
    //   1. Posts BridgePayload to the TSFN → the closure runs on the JS thread,
    //      converting it into JS call args (source: string, path: string).
    //   2. napi-rs calls the user's JS function with those args.
    //   3. `cb` is called on the JS thread with the JS return value (JsUnknown).
    //   4. `cb` coerces the return to a UTF-8 string and sends it through the channel.
    //
    // Lock the Mutex to access the TSFN. Since we hold the lock only briefly
    // (just to enqueue the call), this doesn't create long-lived contention.
    let tsfn_guard = match bridge.tsfn.lock() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[bun-js-beforeparse] Mutex poisoned: {e}");
            return;
        }
    };

    // CalleeHandled variant: call_with_return_value takes Result<T>.
    // NOTE: CalleeHandled prepends null as the first JS arg (Node.js error-first convention).
    // The JS wrapper in index.ts wraps the user callback to skip that first arg.
    tsfn_guard.call_with_return_value(
        Ok(BridgePayload { source, path }),
        ThreadsafeFunctionCallMode::Blocking,
        move |js_result: JsUnknown| -> napi::Result<()> {
            // Clean idiomatic chain: JsUnknown → JsString → JsStringUtf8 → String
            let result = js_result
                .coerce_to_string()
                .and_then(|js_str| js_str.into_utf8())
                .and_then(|utf8| utf8.into_owned());
            match result {
                Ok(s) => { let _ = tx.send(s); }
                Err(e) => {
                    eprintln!("[bun-js-beforeparse] JS transform return coercion failed: {e}");
                    let _ = tx.send(String::new()); // empty = keep original source
                }
            }
            Ok(())
        },
    );

    // BLOCK this Bun worker thread until the JS callback sends the result back.
    match rx.recv() {
        Ok(transformed) if !transformed.is_empty() => {
            handle.set_output_source_code(transformed, loader);
        }
        Ok(_) => {
            // Empty string = JS coercion failed — leave handle unchanged (original source).
        }
        Err(_) => {
            // Channel disconnected — JS threw or TSFN was dropped.
            eprintln!("[bun-js-beforeparse] channel disconnected before result was sent");
        }
    }
}
