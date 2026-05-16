//! Native PipeWire EQ integration.
//!
//! Replaces `pw-cli` and `pw-link` subprocess calls with direct PipeWire API usage
//! via the `pipewire` Rust crate. A dedicated PipeWire thread runs a `MainLoop`,
//! maintains a registry cache, and processes commands sent from the engine thread.

use pipewire::registry::GlobalObject;
use pipewire::types::ObjectType;
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex, OnceLock};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Registry cache — shared between PW thread and callers
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) struct NodeInfo {
    pub(crate) id: u32,
    pub(crate) name: String,
}

#[derive(Clone, Debug)]
pub(crate) struct LinkInfo {
    pub(crate) id: u32,
    pub(crate) output_node: String,
    pub(crate) output_port: String,
    pub(crate) input_node: String,
    pub(crate) input_port: String,
}

#[derive(Default, Debug)]
pub(crate) struct RegistryCache {
    pub(crate) nodes: HashMap<u32, NodeInfo>,
    pub(crate) links: HashMap<u32, LinkInfo>,
}

// ---------------------------------------------------------------------------
// Command types sent from caller thread -> PW thread
// ---------------------------------------------------------------------------

enum PwCommand {
    LoadModule {
        module_name: String,
        module_args: String,
        reply: std::sync::mpsc::Sender<Result<u32, String>>,
    },
    DestroyGlobal {
        global_id: u32,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    CreateLink {
        output_node: String,
        output_port: String,
        input_node: String,
        input_port: String,
        reply: std::sync::mpsc::Sender<Result<u32, String>>,
    },
    #[allow(dead_code)]
    Shutdown,
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

struct NativeEqState {
    cmd_sender: pipewire::channel::Sender<PwCommand>,
    cache: Arc<Mutex<RegistryCache>>,
    _thread: std::thread::JoinHandle<()>,
}

static NATIVE_STATE: OnceLock<Mutex<Option<NativeEqState>>> = OnceLock::new();

fn get_state_lock() -> &'static Mutex<Option<NativeEqState>> {
    NATIVE_STATE.get_or_init(|| Mutex::new(None))
}

pub(crate) fn is_available() -> bool {
    get_state_lock()
        .lock()
        .ok()
        .map(|guard| guard.is_some())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Start the native PipeWire thread and connect to the server.
/// Safe to call multiple times — subsequent calls are no-ops.
pub(crate) fn init() -> Result<(), String> {
    let lock = get_state_lock();
    let mut guard = lock.lock().map_err(|e| format!("lock poisoned: {}", e))?;
    if guard.is_some() {
        return Ok(()); // already initialized
    }

    let cache = Arc::new(Mutex::new(RegistryCache::default()));
    let cache_for_thread = Arc::clone(&cache);

    // Channel for sending commands into the PipeWire mainloop.
    let (pw_sender, pw_receiver) = pipewire::channel::channel::<PwCommand>();

    // Barrier to synchronize startup.
    let (startup_tx, startup_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    let thread = std::thread::Builder::new()
        .name("native-eq-pw".into())
        .spawn(move || {
            if let Err(e) = pw_thread_main(pw_receiver, cache_for_thread, startup_tx) {
                error!("native PipeWire thread exited with error: {}", e);
            }
        })
        .map_err(|e| format!("failed to spawn PipeWire thread: {}", e))?;

    // Wait for the thread to report success or failure.
    startup_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "native PipeWire thread startup timed out".to_string())?
        .map_err(|e| format!("native PipeWire thread startup failed: {}", e))?;

    *guard = Some(NativeEqState {
        cmd_sender: pw_sender,
        cache,
        _thread: thread,
    });

    info!("native PipeWire EQ backend initialized");
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API: commands sent to the PW thread
// ---------------------------------------------------------------------------

/// Load `libpipewire-module-filter-chain` with the given args string.
/// Returns the module's global ID on success.
pub(crate) fn load_filter_chain(module_args: &str) -> Result<u32, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    send_cmd(PwCommand::LoadModule {
        module_name: "libpipewire-module-filter-chain".to_string(),
        module_args: module_args.to_string(),
        reply: tx,
    })?;
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("load_filter_chain reply timeout: {}", e))?
}

/// Destroy a PipeWire global object by its ID.
pub(crate) fn destroy_global(global_id: u32) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    send_cmd(PwCommand::DestroyGlobal {
        global_id,
        reply: tx,
    })?;
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("destroy_global reply timeout: {}", e))?
}

/// Create a link between output and input ports.
pub(crate) fn create_link(
    output_node: &str,
    output_port: &str,
    input_node: &str,
    input_port: &str,
) -> Result<u32, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    send_cmd(PwCommand::CreateLink {
        output_node: output_node.to_string(),
        output_port: output_port.to_string(),
        input_node: input_node.to_string(),
        input_port: input_port.to_string(),
        reply: tx,
    })?;
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("create_link reply timeout: {}", e))?
}

/// Find a node ID by its `node.name` property from the registry cache.
pub(crate) fn find_node_by_name(name: &str) -> Option<u32> {
    let lock = get_state_lock();
    let guard = lock.lock().ok()?;
    let state = guard.as_ref()?;
    let cache = state.cache.lock().ok()?;
    cache
        .nodes
        .values()
        .find(|node| node.name == name)
        .map(|node| node.id)
}

/// List all current links involving `effect_output.stratum_eq` output ports.
pub(crate) fn list_eq_output_links() -> Vec<LinkInfo> {
    let lock = get_state_lock();
    let guard = match lock.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let state = match guard.as_ref() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let cache = match state.cache.lock() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    cache
        .links
        .values()
        .filter(|link| link.output_node == "effect_output.stratum_eq")
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Internal: command dispatch helper
// ---------------------------------------------------------------------------

fn send_cmd(cmd: PwCommand) -> Result<(), String> {
    let lock = get_state_lock();
    let guard = lock
        .lock()
        .map_err(|_| "native PipeWire state lock poisoned".to_string())?;
    let state = guard
        .as_ref()
        .ok_or_else(|| "native PipeWire backend not initialized".to_string())?;
    state
        .cmd_sender
        .send(cmd)
        .map_err(|_| "failed to send command to PipeWire thread".to_string())
}

// ---------------------------------------------------------------------------
// PipeWire thread main function
// ---------------------------------------------------------------------------

fn pw_thread_main(
    pw_receiver: pipewire::channel::Receiver<PwCommand>,
    cache: Arc<Mutex<RegistryCache>>,
    startup_tx: std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    pipewire::init();

    let mainloop = match pipewire::main_loop::MainLoopRc::new(None) {
        Ok(ml) => ml,
        Err(e) => {
            let msg = format!("failed to create mainloop: {}", e);
            let _ = startup_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };

    let context = match pipewire::context::ContextRc::new(&mainloop, None) {
        Ok(ctx) => ctx,
        Err(e) => {
            let msg = format!("failed to create context: {}", e);
            let _ = startup_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };

    let core = match context.connect_rc(None) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("failed to connect to core: {}", e);
            let _ = startup_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };

    let registry = match core.get_registry_rc() {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("failed to get registry: {}", e);
            let _ = startup_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };

    // Set up registry listener to populate the cache.
    let cache_on_global = Arc::clone(&cache);
    let cache_on_remove = Arc::clone(&cache);

    let _reg_listener = registry
        .add_listener_local()
        .global(move |global| {
            update_cache_on_global(&cache_on_global, global);
        })
        .global_remove(move |id| {
            if let Ok(mut c) = cache_on_remove.lock() {
                c.nodes.remove(&id);
                c.links.remove(&id);
            }
        })
        .register();

    // Attach the command receiver to the mainloop.
    let mainloop_for_quit = mainloop.clone();
    let core_for_cmds = core.clone();
    let registry_for_cmds = registry.clone();
    let context_ptr = context.as_raw_ptr();

    let _receiver_attachment = pw_receiver.attach(mainloop.loop_(), move |cmd| {
        match cmd {
            PwCommand::LoadModule {
                module_name,
                module_args,
                reply,
            } => {
                let result = load_module_ffi(context_ptr, &module_name, &module_args);
                let _ = reply.send(result);
            }
            PwCommand::DestroyGlobal { global_id, reply } => {
                let result = destroy_global_safe(&registry_for_cmds, global_id);
                let _ = reply.send(result);
            }
            PwCommand::CreateLink {
                output_node,
                output_port,
                input_node,
                input_port,
                reply,
            } => {
                let result = create_link_native(
                    &core_for_cmds,
                    &output_node,
                    &output_port,
                    &input_node,
                    &input_port,
                );
                let _ = reply.send(result);
            }
            PwCommand::Shutdown => {
                mainloop_for_quit.quit();
            }
        }
    });

    // Signal that we started successfully.
    let _ = startup_tx.send(Ok(()));

    // Run until quit is called.
    mainloop.run();

    Ok(())
}

// ---------------------------------------------------------------------------
// Registry cache helpers
// ---------------------------------------------------------------------------

fn update_cache_on_global(
    cache: &Arc<Mutex<RegistryCache>>,
    global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>,
) {
    let props = match global.props {
        Some(p) => p,
        None => return,
    };

    match global.type_ {
        ObjectType::Node => {
            if let Some(name) = props.get(&pipewire::keys::NODE_NAME) {
                if let Ok(mut c) = cache.lock() {
                    c.nodes.insert(
                        global.id,
                        NodeInfo {
                            id: global.id,
                            name: name.to_string(),
                        },
                    );
                }
            }
        }
        ObjectType::Link => {
            let output_node = props.get("link.output.node").unwrap_or("").to_string();
            let output_port = props.get("link.output.port").unwrap_or("").to_string();
            let input_node = props.get("link.input.node").unwrap_or("").to_string();
            let input_port = props.get("link.input.port").unwrap_or("").to_string();

            if let Ok(mut c) = cache.lock() {
                c.links.insert(
                    global.id,
                    LinkInfo {
                        id: global.id,
                        output_node,
                        output_port,
                        input_node,
                        input_port,
                    },
                );
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// FFI: pw_context_load_module (not exposed by the safe pipewire crate)
// ---------------------------------------------------------------------------

fn load_module_ffi(
    context_ptr: *mut pipewire::sys::pw_context,
    module_name: &str,
    module_args: &str,
) -> Result<u32, String> {
    let c_name =
        CString::new(module_name).map_err(|_| "module name contains null byte".to_string())?;
    let c_args =
        CString::new(module_args).map_err(|_| "module args contain null byte".to_string())?;

    let module_ptr = unsafe {
        pipewire::sys::pw_context_load_module(
            context_ptr,
            c_name.as_ptr(),
            c_args.as_ptr(),
            std::ptr::null_mut(), // no extra properties
        )
    };

    if module_ptr.is_null() {
        return Err(
            "pw_context_load_module returned null — module failed to load".to_string(),
        );
    }

    // Extract the module's global ID from the impl_module structure.
    let info = unsafe { pipewire::sys::pw_impl_module_get_info(module_ptr) };
    if info.is_null() {
        // Module loaded but no info available yet; return 0 as sentinel.
        // The caller should poll registry for the node instead.
        warn!("native: module loaded but info not yet available");
        return Ok(0);
    }

    let id = unsafe { (*info).id };
    Ok(id)
}

// ---------------------------------------------------------------------------
// Safe: registry destroy_global
// ---------------------------------------------------------------------------

fn destroy_global_safe(
    registry: &pipewire::registry::Registry,
    global_id: u32,
) -> Result<(), String> {
    let result = registry.destroy_global(global_id);

    match result.into_result() {
        Ok(_) => Ok(()),
        Err(e) => Err(format!(
            "destroy_global failed for global {}: {}",
            global_id, e
        )),
    }
}

// ---------------------------------------------------------------------------
// Native link creation via Core::create_object
// ---------------------------------------------------------------------------

fn create_link_native(
    core: &pipewire::core::Core,
    output_node: &str,
    output_port: &str,
    input_node: &str,
    input_port: &str,
) -> Result<u32, String> {
    let props = pipewire::properties::properties! {
        "link.output.node" => output_node,
        "link.output.port" => output_port,
        "link.input.node" => input_node,
        "link.input.port" => input_port,
        "object.linger" => "true"
    };

    let link: pipewire::link::Link = core
        .create_object("link-factory", &props)
        .map_err(|e| format!("failed to create link: {}", e))?;

    // Get the proxy ID so we can track/destroy this link later.
    let proxy_id = {
        use pipewire::proxy::ProxyT;
        link.upcast_ref().id()
    };



    // Intentionally leak the link proxy — we don't want it dropped (which would
    // destroy the link on the server). Links are destroyed via destroy_global.
    std::mem::forget(link);

    Ok(proxy_id)
}
