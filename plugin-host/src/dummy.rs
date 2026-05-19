use std::path::Path;

use anyhow::Result;
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

pub fn dummy_component_bytes() -> Result<Vec<u8>> {
    let mut resolve = Resolve::default();
    let wit_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../wit");
    let (package, _) = resolve.push_dir(&wit_dir)?;
    let world = resolve.select_world(package, Some("messenger-plugin"))?;

    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)?;

    ComponentEncoder::default()
        .validate(true)
        .module(&module)?
        .encode()
}
