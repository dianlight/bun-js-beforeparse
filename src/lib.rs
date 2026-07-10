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
//!     posts (source, path) via TSFN           gets the String result
//!     blocks on rx.recv()                     sender.send(result)
//!     writes result → OnBeforeParseResult

#![deny(clippy::all)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::panic;
use std::sync::{Arc, mpsc};

use bun_native_plugin::{define_bun_plugin, OnBeforeParse};

// Export the BUN_PLUGIN_NAME symbol that Bun requires to validate this NAPI module.
// Without this, build.onBeforeParse() throws "must be a Napi module which exports BUN_PLUGIN_NAME".
define_bun_plugin!("bun-js-beforeparse");
use napi::threadsafe_function::{
    ThreadsafeCallContext, ThreadsafeFunction, ThreadsafeFunctionCallMode,
};
use napi::bindgen_prelude::{FnArgs, *};
use napi::Env;
use napi_derive::napi;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Holds the ThreadsafeFunction wrapped in Arc so the extern "C" hook can use
/// it from concurrent worker threads.
///
/// In napi-rs v3, ThreadsafeFunction<T> is Clone + Send + Sync natively, so no
/// Mutex is needed — call_with_return_value takes &self.
///
/// Generic parameters:
///   T                = (String, String)         →  (source, path) posted from worker threads
///   Return           = String                   →  JS callback return value ('static required)
///   CallJsBackArgs   = FnArgs<(String, String)> →  spreads the tuple into 2 positional JS args
///                                                   (plain (String,String) would be one arg)
///   const CalleeHandled = false  →  JS called as fn(source, path) directly, no null prefix
///   const Weak          = true   →  does not prevent process exit (weak reference)
///   const MaxQueueSize  = 0      →  unlimited queue
///
/// Using Weak=true means release_bridge() can be a no-op — the process exits naturally
/// once the event loop drains. For Bun.serve() (long-running) this is ideal; for Bun.build()
/// one-shot builds the process exits quickly after the build completes regardless.
pub struct BridgeFn {
    tsfn: ThreadsafeFunction<
        (String, String),
        String,
        FnArgs<(String, String)>,
        napi::Status,
        false,   // CalleeHandled=false: no null error-first arg
        true,    // Weak=true: does not hold event loop open
    >,
}

// SAFETY: ThreadsafeFunction in napi-rs v3 is Send+Sync; Arc<BridgeFn> is Send+Sync.
unsafe impl Send for BridgeFn {}
unsafe impl Sync for BridgeFn {}

// ─── NAPI export: create_bridge ─────────────────────────────────────────────

/// Called from TypeScript's `jsBridge(fn)` to register the user's JS callback.
///
/// Returns `External<Arc<BridgeFn>>` — an opaque value passed as `external` in
/// `build.onBeforeParse(matcher, { napiModule, symbol, external })`.
/// Bun hands this pointer back to `bun_js_bridge_dispatch` on every file.
///
/// Uses napi-rs v3's Function builder. The JS callback is called as fn(source, path)
/// with callee_handled=false (no null error-first arg). Return type is String.
#[napi]
pub fn create_bridge(
    callback: Function<(String, String), String>,
) -> Result<External<Arc<BridgeFn>>> {
    let tsfn = callback
        .build_threadsafe_function::<(String, String)>()
        // callee_handled::<false> = JS callback receives (source, path) directly.
        // No null error-first arg is prepended. js/index.ts passes fn through unchanged.
        .callee_handled::<false>()
        .weak::<true>()             // weak ref: does not prevent process exit by itself
        .max_queue_size::<0>()      // unlimited queue; each call blocks the worker anyway
        .build_callback(
            |ctx: ThreadsafeCallContext<(String, String)>| -> Result<FnArgs<(String, String)>> {
                // ctx.value is the (source, path) tuple posted from the worker thread.
                // FnArgs wraps the tuple so JsValuesTupleIntoVec spreads it into two
                // separate positional JS args: fn(source, path).
                // A plain (String, String) return would pass ONE tuple-object arg instead.
                Ok(FnArgs { data: ctx.value })
            },
        )?;

    // Wrap in Arc<BridgeFn> so dispatch can be called from concurrent worker threads.
    Ok(External::new(Arc::new(BridgeFn { tsfn })))
}

/// Call this after your build completes if you need explicit cleanup.
///
/// With Weak=true (set in create_bridge), the TSFN does not hold a strong event-loop
/// reference — the process exits freely once the event loop drains. This function
/// exists for API compatibility; it is a no-op in napi-rs v3.
///
/// In napi v3, External parameters are received as `&External<T>` (borrowed from JS heap).
#[napi]
pub fn release_bridge(_bridge: &External<Arc<BridgeFn>>) -> Result<()> {
    // No-op: the TSFN was built with weak::<true>() and does not hold the event loop open.
    // The TSFN is released when the External<Arc<BridgeFn>> is GC'd by the JS runtime.
    Ok(())
}

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
    // napi v3 External<T> stores data as *mut External<T> (with a TypeId tag).
    // We must use External::<Arc<BridgeFn>>::inner_from_raw(ptr) to read through
    // the wrapper correctly. Direct casting segfaults.
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

    // call_with_return_value in napi-rs v3:
    //   1. Posts (source, path) tuple to the TSFN → build_callback spreads it into two
    //      positional JS call args: fn(source, path).
    //   2. napi-rs calls the JS callback. Return = String.
    //   3. Closure is called on the JS thread with Result<String>:
    //      Ok(s)  → JS returned a string; send it through the channel.
    //      Err(e) → JS threw or non-string return; fall back to original source.
    //
    // v3 TSFN is Send+Sync; call_with_return_value takes &self — no Mutex needed.
    // callee_handled=false → pass the tuple directly (no Result/Ok() wrapping).
    bridge.tsfn.call_with_return_value(
        (source, path),
        ThreadsafeFunctionCallMode::Blocking,
        // v3 closure: FnOnce(Result<Return>, Env) -> Result<()>. _env unused here.
        move |ret: Result<String>, _env: Env| -> Result<()> {
            match ret {
                Ok(s) => { let _ = tx.send(s); }
                Err(e) => {
                    eprintln!("[bun-js-beforeparse] JS transform failed: {e}");
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
            // Empty string = JS threw or non-string return — leave handle unchanged (original source).
        }
        Err(_) => {
            // Channel disconnected — TSFN was aborted/dropped before result was sent.
            eprintln!("[bun-js-beforeparse] channel disconnected before result was sent");
        }
    }
}
