use std::path::Path;

use anyhow::{Context, Result};
use wasmtime::{
    Engine, Store,
    component::{Component, Linker},
};

use crate::{
    bindings::{
        MessengerPlugin,
        exports::pandere::messenger::plugin::{
            AuthStatus, PluginError, PluginMetadata, RetentionHints,
        },
    },
    runtime_host::RuntimeHost,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentProbe {
    pub instantiated: bool,
}

pub struct PluginHost {
    engine: Engine,
    linker: Linker<RuntimeHost>,
}

impl PluginHost {
    pub fn new(engine: Engine) -> Result<Self> {
        let mut linker = Linker::new(&engine);
        MessengerPlugin::add_to_linker(&mut linker, |host: &mut RuntimeHost| host)
            .context("failed to add generated host bindings to linker")?;

        Ok(Self { engine, linker })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn load_component_from_bytes(&self, bytes: &[u8]) -> Result<Component> {
        Component::new(&self.engine, bytes).context("failed to compile component bytes")
    }

    pub fn load_component_from_file(&self, path: impl AsRef<Path>) -> Result<Component> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read component `{}`", path.display()))?;
        self.load_component_from_bytes(&bytes)
    }

    pub fn instantiate(&self, component: &Component, host: RuntimeHost) -> Result<LoadedPlugin> {
        let mut store = Store::new(&self.engine, host);
        let world = MessengerPlugin::instantiate(&mut store, component, &self.linker)
            .context("failed to instantiate messenger plugin component")?;

        Ok(LoadedPlugin { store, world })
    }

    pub fn probe_component(
        &self,
        component: &Component,
        host: RuntimeHost,
    ) -> Result<ComponentProbe> {
        let _plugin = self.instantiate(component, host)?;
        Ok(ComponentProbe { instantiated: true })
    }
}

pub struct LoadedPlugin {
    store: Store<RuntimeHost>,
    world: MessengerPlugin,
}

impl LoadedPlugin {
    pub fn host(&self) -> &RuntimeHost {
        self.store.data()
    }

    pub fn host_mut(&mut self) -> &mut RuntimeHost {
        self.store.data_mut()
    }

    pub fn metadata(&mut self) -> Result<PluginMetadata> {
        let world = &self.world;
        let store = &mut self.store;
        world
            .pandere_messenger_plugin()
            .call_metadata(store)
            .context("plugin metadata call failed")
    }

    pub fn auth_status(&mut self) -> Result<std::result::Result<AuthStatus, PluginError>> {
        let world = &self.world;
        let store = &mut self.store;
        world
            .pandere_messenger_plugin()
            .call_get_auth_status(store)
            .context("plugin auth-status call failed")
    }

    pub fn retention_hints(&mut self) -> Result<RetentionHints> {
        let world = &self.world;
        let store = &mut self.store;
        world
            .pandere_messenger_plugin()
            .call_get_retention_hints(store)
            .context("plugin retention-hints call failed")
    }
}
