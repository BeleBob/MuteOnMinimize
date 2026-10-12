//! Tracks PipeWire output streams and soft-mutes the ones that belong to
//! muted processes, fading them out and back in over a second.
//!
//! Streams are silenced through `softMute` in the node's Props rather than
//! `mute`: WirePlumber persists `mute` per application, so an app closed while
//! minimized would start muted next time. `softMute` is not persisted, and
//! neither is `softVolumes`, which the fades step through. `softVolumes`
//! replaces the user's volume rather than scaling it, so each step sets the
//! user's volume times the fade gain. PipeWire's own volume ramp properties
//! have no effect on stream nodes, hence the timer.
//!
//! The catch is that any volume or mute change by the user (and WirePlumber
//! restoring a new stream) silently clears both, while the node keeps
//! reporting them as set. So the reported values are useless, and instead
//! every Props change we did not cause ourselves re-applies ours. Each
//! `set_param` of ours produces exactly one params change on the node; those
//! echoes are counted and skipped so that re-applying does not loop.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::io::Cursor;
use std::rc::Rc;
use std::time::Duration;

use pipewire as pw;
use pw::loop_::TimerSource;
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

/// How long a fade out or in takes, and how often its volume is updated.
const FADE: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(20);

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
    /// Whether the stream should be silent.
    wanted: bool,
    /// Fade position: 1 is the user's volume, untouched; 0 is silent.
    level: f32,
    /// Params changes still expected from our own `set_param` calls.
    pending_echoes: u32,
    /// For each Props enumeration in flight, whether the change that
    /// triggered it was someone else's. The result's own sequence number is
    /// assigned by the protocol, so it cannot carry this.
    enums_in_flight: VecDeque<bool>,
    blind_reapplies: u32,
}

impl Stream {
    fn target(&self) -> f32 {
        if self.wanted { 0.0 } else { 1.0 }
    }

    fn is_fading(&self) -> bool {
        self.level != self.target()
    }

    fn user_mute(&self) -> bool {
        self.user_props.as_ref().is_some_and(|p| p.mute)
    }

    /// The per-channel volumes the user set, as softVolumes would be without
    /// us; `None` until the node has reported them.
    fn user_volumes(&self) -> Option<Vec<f32>> {
        let props = self.user_props.as_ref()?;
        let volume = props.volume.unwrap_or(1.0);
        (!props.channel_volumes.is_empty())
            .then(|| props.channel_volumes.iter().map(|v| v * volume).collect())
    }

    fn set_props(&mut self, properties: Vec<Property>) {
        self.pending_echoes += 1;
        let value = Value::Object(Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Props,
            id: spa::sys::SPA_PARAM_Props,
            properties,
        });
        let bytes = PodSerializer::serialize(Cursor::new(Vec::new()), &value)
            .expect("serializing a Props object cannot fail")
            .0
            .into_inner();
        let pod = Pod::from_bytes(&bytes).expect("serializer produced a valid pod");
        self.node.set_param(ParamType::Props, 0, pod);
    }

    /// Applies the current fade level to the node.
    fn apply(&mut self) {
        let soft_mute = |on| Property::new(spa::sys::SPA_PROP_softMute, Value::Bool(on));
        let soft_volumes = |volumes: Vec<f32>| {
            Property::new(
                spa::sys::SPA_PROP_softVolumes,
                Value::ValueArray(ValueArray::Float(volumes)),
            )
        };
        let user_mute = self.user_mute();
        let user_volumes = self.user_volumes();
        if self.level <= 0.0 {
            self.set_props(vec![soft_mute(true)]);
        } else if self.level >= 1.0 {
            // Back to exactly what the user set, including their own mute.
            let mut properties = vec![soft_mute(user_mute)];
            properties.extend(user_volumes.map(soft_volumes));
            self.set_props(properties);
        } else if let Some(volumes) = user_volumes {
            // Cubic, like PulseAudio's volume scale, so the fade sounds even.
            let gain = self.level.powi(3);
            let volumes = volumes.into_iter().map(|v| v * gain).collect();
            self.set_props(vec![soft_mute(user_mute), soft_volumes(volumes)]);
        }
    }

    /// Moves the fade toward its target by `delta`; returns whether it is
    /// still under way.
    fn step(&mut self, delta: f32) -> bool {
        if !self.is_fading() {
            return false;
        }
        self.level = if self.wanted {
            (self.level - delta).max(0.0)
        } else {
            (self.level + delta).min(1.0)
        };
        self.apply();
        self.is_fading()
    }

    /// Updates whether the stream should be silent. With `fade`, a change is
    /// left to the timer when the volumes are known and the user has not
    /// muted it; otherwise it takes effect at once.
    fn sync(&mut self, roots: &HashSet<u32>, fade: bool) {
        let wanted = self.pid.is_some_and(|pid| proctree::in_trees(pid, roots));
        if wanted == self.wanted {
            return;
        }
        self.wanted = wanted;
        self.blind_reapplies = 0;
        eprintln!(
            "{} \"{}\" (pid {:?})",
            if wanted { "Muting" } else { "Unmuting" },
            self.name,
            self.pid
        );
        if !(fade && self.user_volumes().is_some() && !self.user_mute()) {
            self.level = self.target();
            self.apply();
        }
    }
}

#[derive(Default)]
struct State {
    streams: HashMap<u32, Stream>,
    /// `pipewire.sec.pid` of every client: the peer PID of its socket.
    client_pids: HashMap<u32, u32>,
    roots: HashSet<u32>,
    /// Drives the fades; armed only while one is under way.
    timer: Option<TimerSource<'static>>,
    ticking: bool,
}

impl State {
    fn set_roots(&mut self, roots: HashSet<u32>) {
        self.roots = roots;
        for stream in self.streams.values_mut() {
            stream.sync(&self.roots, true);
        }
        self.start_ticking();
    }

    fn start_ticking(&mut self) {
        if self.ticking || !self.streams.values().any(Stream::is_fading) {
            return;
        }
        if let Some(timer) = &self.timer {
            let _ = timer.update_timer(Some(TICK), Some(TICK));
            self.ticking = true;
        }
    }

    fn tick(&mut self, expirations: u64) {
        let delta = TICK.as_secs_f32() / FADE.as_secs_f32() * expirations.max(1) as f32;
        let mut fading = false;
        for stream in self.streams.values_mut() {
            fading |= stream.step(delta);
        }
        if !fading && let Some(timer) = &self.timer {
            let _ = timer.update_timer(None, None);
            self.ticking = false;
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
        // Streams appear before their volumes are known, so one that starts
        // while its app is in the background is muted at once, not faded.
        stream.sync(&self.roots, true);
        self.start_ticking();
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
        // Only a stream held silent needs anything: one that is untouched is
        // not ours, and a fading one gets the new volumes on the next tick.
        if !(stream.wanted && stream.level <= 0.0) {
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
        stream.apply();
    }

    /// Gives back every stream we muted, at once.
    fn release_all(&mut self) {
        self.roots.clear();
        for stream in self.streams.values_mut() {
            stream.sync(&self.roots, false);
            if stream.level < 1.0 {
                stream.level = 1.0;
                stream.apply();
            }
        }
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
    // The loop lives as long as the process; leaking it gives the fade timer
    // a 'static lifetime so that it can be kept in the state.
    let main_loop: &'static pw::main_loop::MainLoopRc =
        Box::leak(Box::new(pw::main_loop::MainLoopRc::new(None)?));
    let context = pw::context::ContextRc::new(main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let state = Rc::new(RefCell::new(State::default()));
    let timer = main_loop.loop_().add_timer({
        let state = Rc::downgrade(&state);
        move |expirations| {
            if let Some(state) = state.upgrade() {
                state.borrow_mut().tick(expirations);
            }
        }
    });
    state.borrow_mut().timer = Some(timer);
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
                        wanted: false,
                        level: 1.0,
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
