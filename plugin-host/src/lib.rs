use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use wasmtime::{Config, Engine};
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

pub mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "messenger-plugin",
        trappable_imports: true,
    });
}

use bindings::pandere::messenger::types::{PluginError, SecretRef};

pub fn crate_name() -> &'static str {
    "pandere-plugin-host"
}

pub fn component_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);

    Ok(Engine::new(&config)?)
}

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

#[derive(Debug, Default)]
pub struct HostState {
    sessions: HashMap<String, String>,
    secrets: HashMap<String, String>,
}

impl HostState {
    pub fn load_session(&self, key: &str) -> Option<String> {
        self.sessions.get(key).cloned()
    }

    pub fn store_session(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.sessions.insert(key.into(), value.into());
    }

    pub fn load_secret(&self, secret: &SecretRef) -> std::result::Result<String, PluginError> {
        self.secrets
            .get(&secret.handle)
            .cloned()
            .ok_or_else(|| PluginError::Unsupported("unknown secret handle".into()))
    }

    pub fn store_secret(
        &mut self,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> std::result::Result<SecretRef, PluginError> {
        let label = label.into();
        let handle = format!("secret://{label}");
        self.secrets.insert(handle.clone(), value.into());
        Ok(SecretRef { handle })
    }

    pub fn now_unix_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::component::{Component, Linker};
    use wasmtime::Store;

    use crate::bindings::{
        MessengerPlugin,
        pandere::messenger::{
            host::{Host as GuestHostTrait, HttpRequest, HttpResponse, PluginError as HostPluginError, SecretRef as HostSecretRef},
            types::{Host as TypesHostTrait, Service},
        },
    };

    #[derive(Default)]
    struct TestHost {
        state: HostState,
        logs: Vec<(String, String)>,
    }

    impl GuestHostTrait for TestHost {
        fn now_unix_secs(&mut self) -> anyhow::Result<u64> {
            Ok(self.state.now_unix_secs())
        }

        fn log(&mut self, level: String, message: String) -> anyhow::Result<()> {
            self.logs.push((level, message));
            Ok(())
        }

        fn load_session(&mut self, key: String) -> anyhow::Result<Option<String>> {
            Ok(self.state.load_session(&key))
        }

        fn store_session(&mut self, key: String, value: String) -> anyhow::Result<()> {
            self.state.store_session(key, value);
            Ok(())
        }

        fn load_secret(
            &mut self,
            handle: HostSecretRef,
        ) -> anyhow::Result<std::result::Result<String, HostPluginError>> {
            Ok(self.state.load_secret(&SecretRef {
                handle: handle.handle,
            }))
        }

        fn store_secret(
            &mut self,
            label: String,
            value: String,
        ) -> anyhow::Result<std::result::Result<HostSecretRef, HostPluginError>> {
            Ok(self.state.store_secret(label, value).map(|secret| HostSecretRef {
                handle: secret.handle,
            }))
        }

        fn send_http(
            &mut self,
            request: HttpRequest,
        ) -> anyhow::Result<std::result::Result<HttpResponse, HostPluginError>> {
            let response = HttpResponse {
                status: 200,
                headers: request.headers,
                body: request.body.unwrap_or_default(),
            };
            Ok(Ok(response))
        }
    }

    impl TypesHostTrait for TestHost {}

    #[test]
    fn component_engine_enables_component_model() {
        let _engine = component_engine().expect("component engine should initialize");
    }

    #[test]
    fn host_state_round_trips_session_and_secret_values() {
        let mut host = HostState::default();
        host.store_session("telegram/session", "serialized-session");
        assert_eq!(
            host.load_session("telegram/session").as_deref(),
            Some("serialized-session")
        );

        let secret = host
            .store_secret("telegram-auth", "top-secret")
            .expect("secret should store");
        let loaded = host.load_secret(&secret).expect("secret should load");
        assert_eq!(loaded, "top-secret");
    }

    #[test]
    fn dummy_component_instantiates_via_bindgen() {
        let engine = component_engine().expect("component engine should initialize");
        let component_bytes = dummy_component_bytes().expect("dummy component should encode");
        let component =
            Component::new(&engine, component_bytes).expect("dummy component should compile");

        let mut linker = Linker::new(&engine);
        MessengerPlugin::add_to_linker(&mut linker, |host: &mut TestHost| host)
            .expect("host bindings should link");

        let mut store = Store::new(&engine, TestHost::default());
        let world = MessengerPlugin::instantiate(&mut store, &component, &linker)
            .expect("component should instantiate");

        let guest = world.pandere_messenger_plugin();
        let metadata_error = guest
            .call_metadata(&mut store)
            .expect_err("dummy guest export should trap when called");
        let message = format!("{metadata_error:#}");
        assert!(
            message.contains("unreachable") || message.contains("wasm trap"),
            "unexpected trap error: {message}"
        );

        let _ = Service::Telegram;
    }
}
