//! Multiplayer (Unity Netcode for GameObjects equivalent) over UDP with `renet`.
//!
//! - [`Network`] (a World resource): start a server, host (server + local player) or client;
//!   tracks spawned objects by network id.
//! - [`NetworkManager`] (component): starts a mode on Play and spawns a player prefab for
//!   every client that connects.
//! - [`NetworkObject`]: an object the server spawned for everyone (from a prefab, so every
//!   peer can build it), with an owner.
//! - [`NetworkTransform`]: replicates position/rotation from the authority (the server, or
//!   the owner when `owner_authoritative`, Unity's ClientNetworkTransform) and interpolates
//!   on the other peers.
//! - [`NetworkRigidbody`]: makes the Rigidbody kinematic on peers without authority, so
//!   local physics doesn't fight the replicated transform.
//! - RPCs: [`NetworkExt::server_rpc`] / [`NetworkExt::client_rpc`] deliver a named message
//!   with a value to the object's components (`Component::on_message`).
//!
//! The server is authoritative: only it spawns and despawns.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.netcode.gameobjects@latest>

use oxide_core::reflect::Value;
use oxide_core::{Reflect, debug_log, debug_log_warning};
use oxide_math::{Quaternion, Vector3};
use oxide_physics::Rigidbody;
use oxide_scene::{Component, ComponentRegistry, Ctx, GameObjectId, System, World};
use renet::{ConnectionConfig, DefaultChannel, RenetClient, RenetServer, ServerEvent};
use renet_netcode::{
    ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport, ServerAuthentication,
    ServerConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant, SystemTime};

/// Client id of the host's own player (the host has no network connection to itself).
pub const HOST_CLIENT_ID: u64 = 0;
/// Games with a different id can't connect to each other.
pub const PROTOCOL_ID: u64 = 0x0_0A1D_E001;
/// How far behind the newest state remote objects are shown (smooths jitter).
const INTERPOLATION_DELAY: f32 = 0.1;

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Msg {
    Welcome {
        client_id: u64,
    },
    Spawn {
        id: u64,
        prefab: String,
        owner: u64,
        position: [f32; 3],
        rotation: [f32; 4],
    },
    Despawn {
        id: u64,
    },
    State {
        id: u64,
        position: [f32; 3],
        rotation: [f32; 4],
    },
    Rpc {
        id: u64,
        name: String,
        arg: Value,
    },
}

fn encode(m: &Msg) -> Vec<u8> {
    bincode::serde::encode_to_vec(m, bincode::config::standard()).unwrap_or_default()
}

fn decode(b: &[u8]) -> Option<Msg> {
    bincode::serde::decode_from_slice(b, bincode::config::standard())
        .ok()
        .map(|(m, _)| m)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetMode {
    Offline,
    Server,
    /// Server with a local player.
    Host,
    Client,
}

/// The network session (a World resource).
pub struct Network {
    pub mode: NetMode,
    server: Option<(RenetServer, NetcodeServerTransport)>,
    client: Option<(RenetClient, NetcodeClientTransport)>,
    /// This peer's client id (`HOST_CLIENT_ID` on a host; assigned by the server on clients).
    pub local_client_id: u64,
    /// Network id → object.
    objects: HashMap<u64, GameObjectId>,
    next_id: u64,
    last_update: Instant,
    /// Clients that connected/disconnected this frame (server side).
    pub connected: Vec<u64>,
    pub disconnected: Vec<u64>,
    /// Messages to deliver to local components after the network update.
    outbox: Vec<(u64, String, Value)>,
    /// Extra `ip:port` addresses clients may use to reach this server (a public IP behind
    /// port forwarding, say). Loopback and the LAN address are always accepted.
    pub public_addresses: Vec<String>,
    /// The port the server is listening on (useful after asking for port 0).
    pub port: u16,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            mode: NetMode::Offline,
            server: None,
            client: None,
            local_client_id: HOST_CLIENT_ID,
            objects: HashMap::new(),
            next_id: 1,
            last_update: Instant::now(),
            connected: Vec::new(),
            disconnected: Vec::new(),
            outbox: Vec::new(),
            public_addresses: Vec::new(),
            port: 0,
        }
    }
}

/// This machine's LAN address (no packet is sent: connecting a UDP socket only picks a route).
fn lan_ip() -> Option<std::net::IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip()).filter(|ip| !ip.is_unspecified())
}

fn now_since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
}

impl Network {
    pub fn is_server(&self) -> bool {
        matches!(self.mode, NetMode::Server | NetMode::Host)
    }

    pub fn is_client(&self) -> bool {
        matches!(self.mode, NetMode::Client | NetMode::Host)
    }

    pub fn is_connected_client(&self) -> bool {
        self.client.as_ref().is_some_and(|c| c.0.is_connected())
    }

    /// Clients connected to this server (not counting a host's own player).
    pub fn client_count(&self) -> usize {
        self.server.as_ref().map_or(0, |s| s.0.connected_clients())
    }

    /// Round-trip time to the server in milliseconds (clients).
    pub fn rtt_ms(&self) -> f64 {
        self.client.as_ref().map_or(0.0, |c| c.0.rtt() * 1000.0)
    }

    fn start_listening(&mut self, port: u16, max_clients: usize) -> Result<(), String> {
        let socket = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| format!("port {port}: {e}"))?;
        let port = socket.local_addr().map_err(|e| e.to_string())?.port();
        // Clients are only accepted on addresses listed here, so list every way to reach us.
        let mut public_addresses: Vec<SocketAddr> = vec![([127, 0, 0, 1], port).into()];
        if let Some(ip) = lan_ip() {
            public_addresses.push((ip, port).into());
        }
        public_addresses.extend(
            self.public_addresses
                .iter()
                .filter_map(|a| a.parse::<SocketAddr>().ok()),
        );
        public_addresses.dedup();
        self.port = port;
        let config = ServerConfig {
            current_time: now_since_epoch(),
            max_clients,
            protocol_id: PROTOCOL_ID,
            public_addresses,
            authentication: ServerAuthentication::Unsecure,
        };
        let transport = NetcodeServerTransport::new(config, socket).map_err(|e| e.to_string())?;
        self.server = Some((RenetServer::new(ConnectionConfig::default()), transport));
        self.last_update = Instant::now();
        Ok(())
    }

    /// Dedicated server (no local player).
    pub fn start_server(&mut self, port: u16, max_clients: usize) -> Result<(), String> {
        self.shutdown();
        self.start_listening(port, max_clients)?;
        self.mode = NetMode::Server;
        debug_log!("Server listening on port {}", self.port);
        Ok(())
    }

    /// Server plus a local player (Unity StartHost).
    pub fn start_host(&mut self, port: u16, max_clients: usize) -> Result<(), String> {
        self.shutdown();
        self.start_listening(port, max_clients)?;
        self.mode = NetMode::Host;
        self.local_client_id = HOST_CLIENT_ID;
        debug_log!("Hosting on port {}", self.port);
        Ok(())
    }

    /// Connects to a server at `addr` (`host:port`).
    pub fn start_client(&mut self, addr: &str) -> Result<(), String> {
        self.shutdown();
        let server_addr: SocketAddr = addr
            .parse()
            .or_else(|_| {
                use std::net::ToSocketAddrs;
                addr.to_socket_addrs()
                    .ok()
                    .and_then(|mut a| a.next())
                    .ok_or(())
            })
            .map_err(|_| format!("bad server address `{addr}`"))?;
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        let client_id = now_since_epoch().as_nanos() as u64 | 1;
        let auth = ClientAuthentication::Unsecure {
            protocol_id: PROTOCOL_ID,
            client_id,
            server_addr,
            user_data: None,
        };
        let transport = NetcodeClientTransport::new(now_since_epoch(), auth, socket)
            .map_err(|e| e.to_string())?;
        self.client = Some((RenetClient::new(ConnectionConfig::default()), transport));
        self.local_client_id = client_id;
        self.mode = NetMode::Client;
        self.last_update = Instant::now();
        debug_log!("Connecting to {server_addr}...");
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if let Some((mut s, mut t)) = self.server.take() {
            t.disconnect_all(&mut s);
        }
        if let Some((_, mut t)) = self.client.take() {
            t.disconnect();
        }
        self.mode = NetMode::Offline;
        self.objects.clear();
    }

    pub fn object(&self, network_id: u64) -> Option<GameObjectId> {
        self.objects.get(&network_id).copied()
    }

    fn broadcast(&mut self, reliable: bool, m: &Msg, except: Option<u64>) {
        let Some((s, _)) = self.server.as_mut() else {
            return;
        };
        let ch = if reliable {
            DefaultChannel::ReliableOrdered
        } else {
            DefaultChannel::Unreliable
        };
        match except {
            Some(c) => s.broadcast_message_except(c, ch, encode(m)),
            None => s.broadcast_message(ch, encode(m)),
        }
    }

    fn send_to(&mut self, client: u64, m: &Msg) {
        if let Some((s, _)) = self.server.as_mut() {
            s.send_message(client, DefaultChannel::ReliableOrdered, encode(m));
        }
    }

    fn send_to_server(&mut self, reliable: bool, m: &Msg) {
        if let Some((c, _)) = self.client.as_mut() {
            let ch = if reliable {
                DefaultChannel::ReliableOrdered
            } else {
                DefaultChannel::Unreliable
            };
            c.send_message(ch, encode(m));
        }
    }
}

/// Unity `NetworkObject`: identifies an object across the network.
#[derive(Clone, Debug, Default, Reflect)]
pub struct NetworkObject {
    #[reflect(skip)]
    pub network_id: u64,
    #[reflect(skip)]
    pub owner: u64,
    #[reflect(skip)]
    pub prefab: String,
}

impl Component for NetworkObject {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Replicates position and rotation (Unity `NetworkTransform`).
#[derive(Clone, Debug, Reflect)]
pub struct NetworkTransform {
    /// The owner moves the object and sends it (Unity `ClientNetworkTransform`); else the
    /// server does.
    pub owner_authoritative: bool,
    /// States sent per second.
    pub send_rate: f32,
    pub interpolate: bool,
    #[reflect(skip)]
    #[doc(hidden)]
    pub since_send: f32,
    /// (arrival time, position, rotation)
    #[reflect(skip)]
    #[doc(hidden)]
    pub buffer: VecDeque<(f32, Vector3, Quaternion)>,
}

impl Default for NetworkTransform {
    fn default() -> Self {
        Self {
            owner_authoritative: false,
            send_rate: 30.0,
            interpolate: true,
            since_send: 0.0,
            buffer: VecDeque::new(),
        }
    }
}

impl Component for NetworkTransform {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `NetworkRigidbody`: the body simulates only where the transform has authority;
/// elsewhere it is kinematic and follows the replicated pose. Needs a `NetworkTransform`.
#[derive(Clone, Debug, Default, Reflect)]
pub struct NetworkRigidbody {
    /// The body's own `is_kinematic`, restored when this peer has authority.
    #[reflect(skip)]
    #[doc(hidden)]
    pub was_kinematic: Option<bool>,
}

impl Component for NetworkRigidbody {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Makes `id`'s Rigidbody kinematic without authority, and restores it with authority.
fn sync_rigidbody(world: &mut World, id: GameObjectId, authority: bool) {
    let Some(was) = world
        .get_component::<NetworkRigidbody>(id)
        .map(|n| n.was_kinematic)
    else {
        return;
    };
    let Some(rb) = world.get_component_mut::<Rigidbody>(id) else {
        return;
    };
    let original = was.unwrap_or(rb.is_kinematic);
    rb.is_kinematic = original || !authority;
    if let Some(n) = world.get_component_mut::<NetworkRigidbody>(id) {
        n.was_kinematic = Some(original);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum StartMode {
    /// Started from code (`world.network_mut().start_host(...)`).
    #[default]
    None,
    Host,
    Server,
    Client,
}

/// Unity `NetworkManager`: starts the session and spawns players.
#[derive(Clone, Debug, Reflect)]
pub struct NetworkManager {
    pub start_mode: StartMode,
    /// Server address for clients.
    pub address: String,
    pub port: u32,
    pub max_clients: u32,
    /// Prefab spawned for each player (owned by that player). Empty = none.
    pub player_prefab: String,
    #[reflect(skip)]
    #[doc(hidden)]
    pub players: HashMap<u64, GameObjectId>,
}

impl Default for NetworkManager {
    fn default() -> Self {
        Self {
            start_mode: StartMode::None,
            address: "127.0.0.1".into(),
            port: 7777,
            max_clients: 64,
            player_prefab: String::new(),
            players: HashMap::new(),
        }
    }
}

impl Component for NetworkManager {
    fn start(&mut self, ctx: &mut Ctx) {
        let net = ctx.world.network_mut();
        let r = match self.start_mode {
            StartMode::None => return,
            StartMode::Host => net.start_host(self.port as u16, self.max_clients as usize),
            StartMode::Server => net.start_server(self.port as u16, self.max_clients as usize),
            StartMode::Client => net.start_client(&format!("{}:{}", self.address, self.port)),
        };
        if let Err(e) = r {
            oxide_core::debug_log_error!("NetworkManager: {e}");
        }
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if self.player_prefab.is_empty() || !ctx.world.network().is_server() {
            return;
        }
        let (mode, joined, left) = {
            let net = ctx.world.network_mut();
            (
                net.mode,
                std::mem::take(&mut net.connected),
                std::mem::take(&mut net.disconnected),
            )
        };
        let mut joined = joined;
        if mode == NetMode::Host && !self.players.contains_key(&HOST_CLIENT_ID) {
            joined.push(HOST_CLIENT_ID);
        }
        for c in joined {
            match ctx.world.instantiate_prefab(&self.player_prefab, None) {
                Ok(id) => {
                    ctx.world.network_spawn(id, c);
                    self.players.insert(c, id);
                }
                Err(e) => oxide_core::debug_log_error!("player prefab: {e}"),
            }
        }
        for c in left {
            if let Some(id) = self.players.remove(&c) {
                ctx.world.network_despawn(id);
            }
        }
    }

    fn on_destroy(&mut self, ctx: &mut Ctx) {
        ctx.world.network_mut().shutdown();
    }
}

/// Network calls on the World (and so through `ctx.world`).
pub trait NetworkExt {
    fn network(&self) -> &Network;
    fn network_mut(&mut self) -> &mut Network;
    /// Server: makes `id` (a prefab instance) exist on every client, owned by `owner`.
    fn network_spawn(&mut self, id: GameObjectId, owner: u64) -> Option<u64>;
    /// Server: removes a spawned object everywhere.
    fn network_despawn(&mut self, id: GameObjectId);
    /// Whether this peer owns `id` (its local player, say).
    fn is_owner(&self, id: GameObjectId) -> bool;
    /// Calls `name` with `arg` on the server's copy of `id` (Unity ServerRpc).
    fn server_rpc(&mut self, id: GameObjectId, name: &str, arg: Value);
    /// Server: calls `name` on every client's copy of `id` (Unity ClientRpc).
    fn client_rpc(&mut self, id: GameObjectId, name: &str, arg: Value);
}

impl NetworkExt for World {
    fn network(&self) -> &Network {
        self.resource::<Network>()
    }

    fn network_mut(&mut self) -> &mut Network {
        self.resource_mut::<Network>()
    }

    fn network_spawn(&mut self, id: GameObjectId, owner: u64) -> Option<u64> {
        if !self.network().is_server() {
            debug_log_warning!("only the server can spawn network objects");
            return None;
        }
        let prefab = self.get(id)?.prefab_source.clone().unwrap_or_default();
        if prefab.is_empty() {
            debug_log_warning!(
                "network_spawn: `{}` is not a prefab instance; clients can't build it",
                self.get(id)?.name
            );
        }
        let net = self.resource_mut::<Network>();
        let nid = net.next_id;
        net.next_id += 1;
        net.objects.insert(nid, id);
        if !self.has_component::<NetworkObject>(id) {
            self.add_component(id, NetworkObject::default());
        }
        if let Some(no) = self.get_component_mut::<NetworkObject>(id) {
            no.network_id = nid;
            no.owner = owner;
            no.prefab = prefab.clone();
        }
        let msg = Msg::Spawn {
            id: nid,
            prefab,
            owner,
            position: self.position(id).to_array(),
            rotation: self.rotation(id).to_array(),
        };
        self.network_mut().broadcast(true, &msg, None);
        Some(nid)
    }

    fn network_despawn(&mut self, id: GameObjectId) {
        let Some(nid) = self
            .get_component::<NetworkObject>(id)
            .map(|n| n.network_id)
        else {
            return;
        };
        let net = self.network_mut();
        net.objects.remove(&nid);
        net.broadcast(true, &Msg::Despawn { id: nid }, None);
        self.destroy(id);
    }

    fn is_owner(&self, id: GameObjectId) -> bool {
        let net = self.network();
        match self.get_component::<NetworkObject>(id) {
            Some(no) if no.network_id != 0 => no.owner == net.local_client_id && net.is_client(),
            // Not networked (or offline): everything is local.
            _ => true,
        }
    }

    fn server_rpc(&mut self, id: GameObjectId, name: &str, arg: Value) {
        let Some(nid) = self
            .get_component::<NetworkObject>(id)
            .map(|n| n.network_id)
        else {
            return;
        };
        let net = self.network_mut();
        if net.is_server() {
            net.outbox.push((nid, name.to_owned(), arg));
        } else {
            net.send_to_server(
                true,
                &Msg::Rpc {
                    id: nid,
                    name: name.to_owned(),
                    arg,
                },
            );
        }
    }

    fn client_rpc(&mut self, id: GameObjectId, name: &str, arg: Value) {
        let Some(nid) = self
            .get_component::<NetworkObject>(id)
            .map(|n| n.network_id)
        else {
            return;
        };
        let net = self.network_mut();
        if !net.is_server() {
            return;
        }
        let msg = Msg::Rpc {
            id: nid,
            name: name.to_owned(),
            arg: arg.clone(),
        };
        net.broadcast(true, &msg, None);
        if net.mode == NetMode::Host {
            net.outbox.push((nid, name.to_owned(), arg));
        }
    }
}

/// Whether this peer drives `id`'s transform.
fn has_authority(world: &World, id: GameObjectId, nt: &NetworkTransform) -> bool {
    let net = world.network();
    if nt.owner_authoritative {
        world.is_owner(id)
    } else {
        net.is_server()
    }
}

/// Pumps the network each frame: receive, apply, send.
#[derive(Default)]
pub struct NetworkSystem {
    clock: f32,
    last_pump: Option<Instant>,
}

impl NetworkSystem {
    fn receive(&mut self, world: &mut World) {
        let mut net = world.resources.remove::<Network>().unwrap_or_default();
        let dt = net.last_update.elapsed();
        net.last_update = Instant::now();
        let mut inbox: Vec<(Option<u64>, Msg)> = Vec::new();

        if let Some((server, transport)) = net.server.as_mut() {
            server.update(dt);
            if let Err(e) = transport.update(dt, server) {
                debug_log_warning!("network: {e}");
            }
            while let Some(ev) = server.get_event() {
                match ev {
                    ServerEvent::ClientConnected { client_id } => {
                        debug_log!("Client {client_id} connected");
                        net.connected.push(client_id);
                    }
                    ServerEvent::ClientDisconnected { client_id, reason } => {
                        debug_log!("Client {client_id} disconnected: {reason}");
                        net.disconnected.push(client_id);
                    }
                }
            }
            let server = &mut net.server.as_mut().expect("server").0;
            for c in server.clients_id() {
                for ch in [
                    u8::from(DefaultChannel::ReliableOrdered),
                    u8::from(DefaultChannel::Unreliable),
                ] {
                    while let Some(b) = server.receive_message(c, ch) {
                        if let Some(m) = decode(&b) {
                            inbox.push((Some(c), m));
                        }
                    }
                }
            }
        }
        let mut lost = false;
        if let Some((client, transport)) = net.client.as_mut() {
            client.update(dt);
            if let Err(e) = transport.update(dt, client) {
                debug_log_warning!("Disconnected from server: {e}");
                lost = true;
            } else if client.is_disconnected() {
                debug_log_warning!("Disconnected from server");
                lost = true;
            }
            for ch in [
                u8::from(DefaultChannel::ReliableOrdered),
                u8::from(DefaultChannel::Unreliable),
            ] {
                while let Some(b) = client.receive_message(ch) {
                    if let Some(m) = decode(&b) {
                        inbox.push((None, m));
                    }
                }
            }
        }

        // New clients get every spawned object.
        for c in net.connected.clone() {
            net.send_to(c, &Msg::Welcome { client_id: c });
            let spawned: Vec<(u64, GameObjectId)> =
                net.objects.iter().map(|(k, v)| (*k, *v)).collect();
            for (nid, id) in spawned {
                let Some(no) = world.get_component::<NetworkObject>(id) else {
                    continue;
                };
                let m = Msg::Spawn {
                    id: nid,
                    prefab: no.prefab.clone(),
                    owner: no.owner,
                    position: world.position(id).to_array(),
                    rotation: world.rotation(id).to_array(),
                };
                net.send_to(c, &m);
            }
        }
        if lost {
            // Like Unity: the client shuts down and its copies of network objects go away.
            let spawned: Vec<GameObjectId> = net.objects.values().copied().collect();
            net.shutdown();
            world.resources.insert(net);
            for id in spawned {
                world.destroy(id);
            }
            return;
        }
        world.resources.insert(net);

        let now = self.clock;
        for (from, m) in inbox {
            self.apply(world, from, m, now);
        }
        let outbox = std::mem::take(&mut world.network_mut().outbox);
        for (nid, name, arg) in outbox {
            if let Some(id) = world.network().object(nid) {
                world.send_message(id, &name, arg);
            }
        }
    }

    fn apply(&mut self, world: &mut World, from: Option<u64>, m: Msg, now: f32) {
        let server = world.network().is_server();
        match m {
            Msg::Welcome { client_id } => {
                if !server {
                    world.network_mut().local_client_id = client_id;
                    debug_log!("Connected as client {client_id}");
                }
            }
            Msg::Spawn {
                id,
                prefab,
                owner,
                position,
                rotation,
            } if !server => {
                if world.network().object(id).is_some() {
                    return;
                }
                let obj = match world.instantiate_prefab(&prefab, None) {
                    Ok(o) => o,
                    Err(e) => {
                        oxide_core::debug_log_error!("network spawn: {e}");
                        return;
                    }
                };
                world.set_position(obj, Vector3::from_array(position));
                world.set_rotation(obj, Quaternion::from_array(rotation));
                if !world.has_component::<NetworkObject>(obj) {
                    world.add_component(obj, NetworkObject::default());
                }
                if let Some(no) = world.get_component_mut::<NetworkObject>(obj) {
                    no.network_id = id;
                    no.owner = owner;
                    no.prefab = prefab;
                }
                world.network_mut().objects.insert(id, obj);
            }
            Msg::Despawn { id } if !server => {
                if let Some(obj) = world.network_mut().objects.remove(&id) {
                    world.destroy(obj);
                }
            }
            Msg::State {
                id,
                position,
                rotation,
            } => {
                let Some(obj) = world.network().object(id) else {
                    return;
                };
                // The server only takes states from the owner of owner-authoritative objects.
                if let Some(sender) = from {
                    let ok = world
                        .get_component::<NetworkObject>(obj)
                        .is_some_and(|n| n.owner == sender)
                        && world
                            .get_component::<NetworkTransform>(obj)
                            .is_some_and(|t| t.owner_authoritative);
                    if !ok {
                        return;
                    }
                    let relay = Msg::State {
                        id,
                        position,
                        rotation,
                    };
                    world.network_mut().broadcast(false, &relay, Some(sender));
                }
                let (p, r) = (
                    Vector3::from_array(position),
                    Quaternion::from_array(rotation),
                );
                if let Some(nt) = world.get_component_mut::<NetworkTransform>(obj) {
                    nt.buffer.push_back((now, p, r));
                    while nt.buffer.len() > 32 {
                        nt.buffer.pop_front();
                    }
                }
            }
            Msg::Rpc { id, name, arg } => {
                let Some(obj) = world.network().object(id) else {
                    return;
                };
                world.send_message(obj, &name, arg);
            }
            _ => {}
        }
    }

    /// Sends states for objects this peer drives; moves the others along their buffers.
    fn replicate(&mut self, world: &mut World, dt: f32) {
        let ids: Vec<(GameObjectId, u64)> = world
            .query::<NetworkObject>()
            .filter(|(_, n)| n.network_id != 0)
            .map(|(id, n)| (id, n.network_id))
            .collect();
        let server = world.network().is_server();
        let render_time = self.clock - INTERPOLATION_DELAY;
        for (id, nid) in ids {
            let Some(nt) = world.get_component::<NetworkTransform>(id).cloned() else {
                continue;
            };
            let authority = has_authority(world, id, &nt);
            sync_rigidbody(world, id, authority);
            if authority {
                let interval = 1.0 / nt.send_rate.max(1.0);
                let since = nt.since_send + dt;
                let send = since >= interval;
                if let Some(t) = world.get_component_mut::<NetworkTransform>(id) {
                    t.since_send = if send { since - interval } else { since };
                }
                if send {
                    let m = Msg::State {
                        id: nid,
                        position: world.position(id).to_array(),
                        rotation: world.rotation(id).to_array(),
                    };
                    let net = world.network_mut();
                    if server {
                        net.broadcast(false, &m, None);
                    } else {
                        net.send_to_server(false, &m);
                    }
                }
                continue;
            }
            // Remote: interpolate between the two states around `render_time`.
            let b = &nt.buffer;
            let Some(&(_, last_p, last_r)) = b.back() else {
                continue;
            };
            let (p, r) = if !nt.interpolate {
                (last_p, last_r)
            } else {
                match b.iter().position(|s| s.0 > render_time) {
                    Some(i) if i > 0 => {
                        let (t0, p0, r0) = b[i - 1];
                        let (t1, p1, r1) = b[i];
                        let k = ((render_time - t0) / (t1 - t0).max(1e-4)).clamp(0.0, 1.0);
                        (p0.lerp(p1, k), r0.slerp(r1, k))
                    }
                    Some(_) => (b[0].1, b[0].2),
                    None => (last_p, last_r),
                }
            };
            world.set_position(id, p);
            world.set_rotation(id, r);
        }
    }

    fn flush(&mut self, world: &mut World) {
        let net = world.network_mut();
        if let Some((server, transport)) = net.server.as_mut() {
            transport.send_packets(server);
        }
        if let Some((client, transport)) = net.client.as_mut()
            && let Err(e) = transport.send_packets(client)
        {
            debug_log_warning!("network: {e}");
        }
    }

    /// One network step outside the engine loop (tests, custom loops).
    pub fn step(&mut self, world: &mut World, dt: f32) {
        self.clock += dt;
        self.receive(world);
        self.replicate(world, dt);
        self.flush(world);
    }
}

impl System for NetworkSystem {
    fn name(&self) -> &'static str {
        "Network"
    }

    fn start(&mut self, world: &mut World) {
        if world.try_resource::<Network>().is_none() {
            world.resources.insert(Network::default());
        }
    }

    /// Pumps every frame, even while paused, so connections don't time out. Leaving Play
    /// mode (scripts stop) ends the session.
    fn always(&mut self, world: &mut World) {
        let now = Instant::now();
        let dt = self.last_pump.map_or(0.0, |t| (now - t).as_secs_f32());
        self.last_pump = Some(now);
        if world
            .try_resource::<Network>()
            .is_none_or(|n| n.mode == NetMode::Offline)
        {
            return;
        }
        if !world.run_scripts {
            world.network_mut().shutdown();
            return;
        }
        self.step(world, dt);
    }
}

/// Registers the components and returns the system the engine runs.
pub fn install(world: &mut World) -> Box<dyn System> {
    register(&mut world.registry);
    if world.try_resource::<Network>().is_none() {
        world.resources.insert(Network::default());
    }
    Box::new(NetworkSystem::default())
}

pub fn register(reg: &mut ComponentRegistry) {
    reg.register::<NetworkManager>("Netcode/Network Manager");
    reg.register::<NetworkObject>("Netcode/Network Object");
    reg.register::<NetworkTransform>("Netcode/Network Transform");
    reg.register::<NetworkRigidbody>("Netcode/Network Rigidbody");
}

#[cfg(test)]
mod tests;
