// napi-build generates the correct linker flags so that the compiled .so/.dylib/.dll
// is loadable as a Node.js NAPI module (sets up the NAPI_MODULE_INIT entry point,
// the necessary exported symbols, and rpath handling).
fn main() {
    napi_build::setup();
}
