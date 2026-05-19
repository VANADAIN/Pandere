pub mod generated {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "messenger-plugin",
        trappable_imports: true,
    });
}

pub use generated::*;
