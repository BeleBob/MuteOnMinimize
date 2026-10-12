//! Tracks PipeWire output streams and soft-mutes the ones that belong to
//! muted processes.
//!
//! Streams are silenced through `softMute` in the node's Props rather than
//! `mute`: WirePlumber persists `mute` per application, so an app closed while
//! minimized would start muted next time. `softMute` is not persisted.
//!
//! The catch is that any volume or mute change by the user (and WirePlumber
//! restoring a new stream) silently clears `softMute`, while the node keeps
//! reporting it as set. So the reported value is useless, and instead every
//! Props change we did not cause ourselves re-applies it. Each `set_param` of
//! ours produces exactly one params change on the node; those echoes are
//! counted and skipped so that re-applying does not loop.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::io::Cursor;
use std::rc::Rc;

use pipewire as pw;
use pw::node::{Node, NodeChangeMask, NodeInfoRef, NodeListener};
use pw::spa;
use pw::types::ObjectType;
use spa::param::ParamType;
use spa::pod::deserialize::PodDeserializer;
use spa::pod::serialize::PodSerializer;
use spa::pod::{Object, Pod, Property, Value, ValueArray};

use crate::proctree;

/// Re-applying more often than this without any visible volume or mute change
/// means the echo accounting is off; stop rather than spin.
const MAX_BLIND_REAPPLIES: u32 = 8;

/// Messages from the D-Bus side to the PipeWire loop.
pub enum Msg {
    /// Mute every stream owned by these processes or their descendants.
    SetMutedPids(HashSet<u32>),
}

/// The user-controlled part of a node's Props.
#[derive(PartialEq)]
struct UserProps {
    mute: bool,
    volume: Option<f32>,
    channel_volumes: Vec<f32>,
}

struct Stream {
    node: Node,
    _listener: NodeListener,
    name: String,
    pid: Option<u32>,
    user_props: Option<UserProps>,
    muted_by_us: bool,
    /// Params changes still expected from our own `set_param` calls.
    pending_echoes: u32,
    /// For each Props enumeration in flight, whether the change that
    /// triggered it was someone else's. The result's own sequence number is
    /// assigned by the protocol, so it cannot carry this.
    enums_in_flight: VecDeque<bool>,
    blind_reapplies: u32,
}

impl Stream {
    fn set_soft_mute(&mut self, on: bool) {
        self.pending_echoes += 1;
        let value = Value::Object(Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Props,
            id: spa::sys::SPA_PARAM_Props,
            properties: vec![Property::new(spa::sys::SPA_PROP_softMute, Value::Bool(on))],
        });
        let bytes = PodSerializer::serialize(Cursor::new(Vec::new()), &value)
            .expect("serializing a fixed Props object cannot fail")
            .0
            .into_inner();
        let pod = Pod::from_bytes(&bytes).expect("serializer produced a valid pod");
        self.node.set_param(ParamType::Props, 0, pod);
    }

    fn sync(&mut self, roots: &HashSet<u32>) {
        let wanted = self.pid.is_some_and(|pid| proctree::in_trees(pid, roots));
        if wanted && !self.muted_by_us {
            self.muted_by_us = true;
            self.blind_reapplies = 0;
            eprintln!("Muting \"{}\" (pid {:?})", self.name, self.pid);
            self.set_soft_mute(true);
        } else if !wanted && self.muted_by_us {
            self.muted_by_us = false;
            eprintln!("Unmuting \"{}\" (pid {:?})", self.name, self.pid);
            // Leave the stream silent if the user muted it themselves.
            let user_mute = self.user_props.as_ref().is_some_and(|p| p.mute);
            self.set_soft_mute(user_mute);
        }
    }
}

#[derive(Default)]
struct State {
    streams: HashMap<u32, Stream>,
    /// `pipewire.sec.pid` of every client: the peer PID of its socket.
    client_pids: HashMap<u32, u32>,
    roots: HashSet<u32>,
}

impl State {
    fn set_roots(&mut self, roots: HashSet<u32>) {
        self.roots = roots;
        for stream in self.streams.values_mut() {
            stream.sync(&self.roots);
        }
    }

    fn on_info(&mut self, id: u32, info: &NodeInfoRef) {
        let Some(stream) = self.streams.get_mut(&id) else {
            return;
        };
        // Later info events (state changes and the like) may carry no or
        // partial props; only look at them when they changed, and never let a
        // missing key clear what we already know.
        if info.change_mask().contains(NodeChangeMask::PROPS)
            && let Some(props) = info.props()
        {
            // Pulse streams carry the app's PID on the node (their client is
            // pipewire-pulse itself); native streams only have it on the client.
            let pid = props
                .get("application.process.id")
                .and_then(|p| p.parse().ok())
                .or_else(|| {
                    let client: u32 = props.get("client.id")?.parse().ok()?;
                    self.client_pids.get(&client).copied()
                });
            if pid.is_some() {
                stream.pid = pid;
            }
            if let Some(name) = props
                .get("application.name")
                .or_else(|| props.get("node.name"))
            {
                stream.name = name.to_owned();
            }
        }
        if info.change_mask().contains(NodeChangeMask::PARAMS) {
            let external = if stream.pending_echoes > 0 {
                stream.pending_echoes -= 1;
                false
            } else {
                true
            };
            stream.enums_in_flight.push_back(external);
            stream
                .node
                .enum_params(0, Some(ParamType::Props), 0, u32::MAX);
        }
        stream.sync(&self.roots);
    }

    fn on_props(&mut self, id: u32, pod: &Pod) {
        let Some(stream) = self.streams.get_mut(&id) else {
            return;
        };
        let Some(new) = parse_user_props(pod) else {
            return;
        };
        // Every enumeration yields exactly one main Props object.
        let external = stream.enums_in_flight.pop_front().unwrap_or(true);
        let changed = stream.user_props.as_ref().is_some_and(|old| *old != new);
        stream.user_props = Some(new);
        if !stream.muted_by_us {
            return;
        }
        // A visible change always needs re-applying, even if it arrived
        // together with one of our echoes.
        if changed {
            stream.blind_reapplies = 0;
        } else if external {
            if stream.blind_reapplies >= MAX_BLIND_REAPPLIES {
                return;
            }
            stream.blind_reapplies += 1;
            if stream.blind_reapplies == MAX_BLIND_REAPPLIES {
                eprintln!(
                    "Stream \"{}\" keeps changing, giving up re-muting it",
                    stream.name
                );
            }
        } else {
            return;
        }
        stream.set_soft_mute(true);
    }

    /// Gives back every stream we muted.
    fn release_all(&mut self) {
        self.set_roots(HashSet::new());
    }
}

/// Reads mute and volumes from a Props object. Returns `None` for the
/// secondary Props object that only carries converter settings.
fn parse_user_props(pod: &Pod) -> Option<UserProps> {
    let (_, Value::Object(object)) = PodDeserializer::deserialize_any_from(pod.as_bytes()).ok()?
    else {
        return None;
    };
    let mut mute = None;
    let mut volume = None;
    let mut channel_volumes = Vec::new();
    for property in object.properties {
        match (property.key, property.value) {
            (spa::sys::SPA_PROP_mute, Value::Bool(on)) => mute = Some(on),
            (spa::sys::SPA_PROP_volume, Value::Float(v)) => volume = Some(v),
            (spa::sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(v))) => {
                channel_volumes = v
            }
            _ => {}
        }
    }
    Some(UserProps {
        mute: mute?,
        volume,
        channel_volumes,
    })
}

/// Runs the PipeWire loop until SIGTERM or SIGINT.
pub fn run(rx: pw::channel::Receiver<Msg>) -> Result<(), Box<dyn Error>> {
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let state = Rc::new(RefCell::new(State::default()));
    let failed = Rc::new(Cell::new(false));
    let pending_quit = Rc::new(Cell::new(None));

    let _core_listener = core
        .add_listener_local()
        .done({
            let main_loop = main_loop.downgrade();
            let pending_quit = pending_quit.clone();
            move |id, seq| {
                if id == pw::core::PW_ID_CORE
                    && Some(seq) == pending_quit.get()
                    && let Some(main_loop) = main_loop.upgrade()
                {
                    main_loop.quit();
                }
            }
        })
        .error({
            let main_loop = main_loop.downgrade();
            let failed = failed.clone();
            move |id, _seq, _res, message| {
                eprintln!("PipeWire error on object {id}: {message}");
                if id == pw::core::PW_ID_CORE {
                    failed.set(true);
                    if let Some(main_loop) = main_loop.upgrade() {
                        main_loop.quit();
                    }
                }
            }
        })
        .register();

    let _registry_listener = registry
        .add_listener_local()
        .global({
            let registry = registry.downgrade();
            let state = state.clone();
            move |global| {
                if global.type_ == ObjectType::Client {
                    if let Some(pid) = global
                        .props
                        .and_then(|p| p.get("pipewire.sec.pid"))
                        .and_then(|p| p.parse().ok())
                    {
                        state.borrow_mut().client_pids.insert(global.id, pid);
                    }
                    return;
                }
                if global.type_ != ObjectType::Node
                    || global.props.and_then(|p| p.get("media.class"))
                        != Some("Stream/Output/Audio")
                {
                    return;
                }
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                let node: Node = match registry.bind(global) {
                    Ok(node) => node,
                    Err(err) => {
                        eprintln!("Cannot bind node {}: {err}", global.id);
                        return;
                    }
                };
                let id = global.id;
                let listener = node
                    .add_listener_local()
                    .info({
                        let state = Rc::downgrade(&state);
                        move |info| {
                            if let Some(state) = state.upgrade() {
                                state.borrow_mut().on_info(id, info);
                            }
                        }
                    })
                    .param({
                        let state = Rc::downgrade(&state);
                        move |_seq, param_type, _index, _next, pod| {
                            if let (ParamType::Props, Some(pod), Some(state)) =
                                (param_type, pod, state.upgrade())
                            {
                                state.borrow_mut().on_props(id, pod);
                            }
                        }
                    })
                    .register();
                state.borrow_mut().streams.insert(
                    id,
                    Stream {
                        node,
                        _listener: listener,
                        name: String::new(),
                        pid: None,
                        user_props: None,
                        muted_by_us: false,
                        pending_echoes: 0,
                        enums_in_flight: VecDeque::new(),
                        blind_reapplies: 0,
                    },
                );
            }
        })
        .global_remove({
            let state = state.clone();
            move |id| {
                let mut state = state.borrow_mut();
                state.streams.remove(&id);
                state.client_pids.remove(&id);
            }
        })
        .register();

    let _receiver = rx.attach(main_loop.loop_(), {
        let state = state.clone();
        move |msg| match msg {
            Msg::SetMutedPids(pids) => state.borrow_mut().set_roots(pids),
        }
    });

    let shutdown = {
        let main_loop = main_loop.downgrade();
        let state = state.clone();
        let core = core.clone();
        move || {
            state.borrow_mut().release_all();
            // Quit once the server has processed the unmutes above.
            match core.sync(0) {
                Ok(seq) => pending_quit.set(Some(seq)),
                Err(_) => {
                    if let Some(main_loop) = main_loop.upgrade() {
                        main_loop.quit();
                    }
                }
            }
        }
    };
    let shutdown = Rc::new(shutdown);
    let _sigterm = main_loop
        .loop_()
        .add_signal_local(pw::loop_::Signal::TERM, {
            let shutdown = shutdown.clone();
            move || shutdown()
        });
    let _sigint = main_loop.loop_().add_signal_local(pw::loop_::Signal::INT, {
        let shutdown = shutdown.clone();
        move || shutdown()
    });

    main_loop.run();

    if failed.get() {
        return Err("lost connection to PipeWire".into());
    }
    Ok(())
}
