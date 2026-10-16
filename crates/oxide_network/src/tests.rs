use crate::*;
use oxide_core::Time;
use oxide_scene::register_builtin;
use std::sync::atomic::{AtomicU32, Ordering};

static HITS: AtomicU32 = AtomicU32::new(0);

/// Counts `Hit` RPCs that arrive on this peer.
#[derive(Clone, Debug, Default, Reflect)]
struct Health {
    hp: f32,
}

impl Component for Health {
    fn on_message(&mut self, _ctx: &mut Ctx, message: &str, arg: &Value) -> bool {
        if message == "Hit" {
            if let Value::Float(d) = arg {
                self.hp -= *d as f32;
            }
            HITS.fetch_add(1, Ordering::SeqCst);
            return true;
        }
        false
    }
}

/// A playing world whose project folder is `dir`.
fn peer(dir: &std::path::Path) -> Peer {
    let mut w = World::new();
    register_builtin(&mut w.registry);
    register(&mut w.registry);
    w.registry.register::<Health>("Test/Health");
    w.registry
        .register::<oxide_physics::Rigidbody>("Physics/Rigidbody");
    w.resources.insert(Time::new());
    w.resources
        .insert(oxide_assets::AssetServer::new(dir.to_path_buf()));
    w.resources.insert(Network::default());
    w.start_scripts();
    (w, NetworkSystem::default())
}

fn project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("oxide-net-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a Player prefab with a NetworkTransform and Health.
fn player_prefab(dir: &std::path::Path, owner_authoritative: bool) {
    let (mut w, _) = peer(dir);
    let p = w.create("Player");
    w.add_component(
        p,
        NetworkTransform {
            owner_authoritative,
            ..Default::default()
        },
    );
    w.add_component(p, Health { hp: 100.0 });
    w.add_component(p, oxide_physics::Rigidbody::default());
    w.add_component(p, NetworkRigidbody::default());
    w.create_prefab(p, "Player.prefab").unwrap();
}

/// Steps both peers ~`frames` times at 60 Hz (real time passes so packets flow).
fn pump(a: &mut Peer, b: &mut Peer, frames: u32) {
    for _ in 0..frames {
        for p in [&mut *a, &mut *b] {
            p.1.step(&mut p.0, 1.0 / 60.0);
            p.0.flush_destroyed();
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

type Peer = (World, NetworkSystem);

fn connect(tag: &str, owner_authoritative: bool) -> (Peer, Peer, std::path::PathBuf) {
    let dir = project(tag);
    player_prefab(&dir, owner_authoritative);
    let mut host = peer(&dir);
    host.0.network_mut().start_host(0, 8).unwrap();
    let port = host.0.network().port;
    let mut client = peer(&dir);
    client
        .0
        .network_mut()
        .start_client(&format!("127.0.0.1:{port}"))
        .unwrap();
    for _ in 0..200 {
        pump(&mut host, &mut client, 1);
        if client.0.network().is_connected_client() && host.0.network().client_count() == 1 {
            break;
        }
    }
    assert!(
        client.0.network().is_connected_client(),
        "client never connected"
    );
    (host, client, dir)
}

fn players(w: &World) -> Vec<(GameObjectId, NetworkObject)> {
    w.query::<NetworkObject>()
        .map(|(id, n)| (id, n.clone()))
        .filter(|(_, n)| n.network_id != 0)
        .collect()
}

#[test]
fn spawn_replicate_rpc_and_despawn() {
    let (mut host, mut client, _) = connect("basic", false);
    let client_id = client.0.network().local_client_id;
    assert_ne!(client_id, HOST_CLIENT_ID);

    // Server spawns a player owned by the client; the client builds it from the prefab.
    let obj = host.0.instantiate_prefab("Player.prefab", None).unwrap();
    host.0.set_position(obj, Vector3::new(1.0, 2.0, 3.0));
    let nid = host.0.network_spawn(obj, client_id).unwrap();
    pump(&mut host, &mut client, 30);
    let remote = client
        .0
        .network()
        .object(nid)
        .expect("client got the spawn");
    assert_eq!(players(&client.0).len(), 1);
    assert!(client.0.is_owner(remote));
    assert!(!host.0.is_owner(obj));
    assert!((client.0.position(remote) - Vector3::new(1.0, 2.0, 3.0)).length() < 1e-3);
    // Server-authoritative body: simulated on the server, kinematic on the client.
    let kinematic = |w: &World, id| {
        w.get_component::<oxide_physics::Rigidbody>(id)
            .unwrap()
            .is_kinematic
    };
    assert!(!kinematic(&host.0, obj));
    assert!(kinematic(&client.0, remote));

    // Server-authoritative movement reaches the client (interpolated, so give it time).
    host.0.set_position(obj, Vector3::new(10.0, 0.0, 0.0));
    pump(&mut host, &mut client, 40);
    let p = client.0.position(remote);
    assert!(
        (p - Vector3::new(10.0, 0.0, 0.0)).length() < 1e-3,
        "client sees {p:?}"
    );

    // A client moving a server-authoritative object is overwritten, not sent.
    client.0.set_position(remote, Vector3::new(-50.0, 0.0, 0.0));
    pump(&mut host, &mut client, 30);
    assert!((host.0.position(obj) - Vector3::new(10.0, 0.0, 0.0)).length() < 1e-3);
    assert!((client.0.position(remote) - Vector3::new(10.0, 0.0, 0.0)).length() < 1e-3);

    // ServerRpc from the client runs on the server's copy; ClientRpc runs on the client's.
    HITS.store(0, Ordering::SeqCst);
    client.0.server_rpc(remote, "Hit", Value::Float(25.0));
    pump(&mut host, &mut client, 20);
    assert_eq!(host.0.get_component::<Health>(obj).unwrap().hp, 75.0);
    host.0.client_rpc(obj, "Hit", Value::Float(5.0));
    pump(&mut host, &mut client, 20);
    assert_eq!(client.0.get_component::<Health>(remote).unwrap().hp, 95.0);
    // A host is also a client, so the ClientRpc ran on its copy too (as in Unity).
    assert_eq!(host.0.get_component::<Health>(obj).unwrap().hp, 70.0);
    assert_eq!(HITS.load(Ordering::SeqCst), 3);

    // Despawn removes it everywhere.
    host.0.network_despawn(obj);
    pump(&mut host, &mut client, 20);
    assert!(client.0.network().object(nid).is_none());
    assert!(!client.0.is_alive(remote));

    // When the server goes away, the client goes offline and drops its network objects.
    let obj = host.0.instantiate_prefab("Player.prefab", None).unwrap();
    let nid = host.0.network_spawn(obj, client_id).unwrap();
    pump(&mut host, &mut client, 30);
    let remote = client.0.network().object(nid).unwrap();
    host.0.network_mut().shutdown();
    for _ in 0..200 {
        pump(&mut host, &mut client, 1);
        if client.0.network().mode == NetMode::Offline {
            break;
        }
    }
    assert_eq!(client.0.network().mode, NetMode::Offline);
    assert!(!client.0.is_alive(remote));
}

#[test]
fn owner_authoritative_transform_and_late_join() {
    let (mut host, mut client, dir) = connect("owner", true);
    let client_id = client.0.network().local_client_id;

    // The host's own player and the client's.
    let mine = host.0.instantiate_prefab("Player.prefab", None).unwrap();
    host.0.network_spawn(mine, HOST_CLIENT_ID).unwrap();
    let theirs = host.0.instantiate_prefab("Player.prefab", None).unwrap();
    let nid = host.0.network_spawn(theirs, client_id).unwrap();
    pump(&mut host, &mut client, 30);
    assert_eq!(players(&client.0).len(), 2);
    let remote = client.0.network().object(nid).unwrap();
    // Owner-authoritative body: simulated by its owner only.
    let kinematic = |w: &World, id| {
        w.get_component::<oxide_physics::Rigidbody>(id)
            .unwrap()
            .is_kinematic
    };
    assert!(!kinematic(&client.0, remote));
    assert!(kinematic(&host.0, theirs));
    assert!(!kinematic(&host.0, mine));

    // The owning client drives its player; the server follows.
    client.0.set_position(remote, Vector3::new(0.0, 0.0, 7.0));
    pump(&mut host, &mut client, 40);
    let p = host.0.position(theirs);
    assert!(
        (p - Vector3::new(0.0, 0.0, 7.0)).length() < 1e-3,
        "server sees {p:?}"
    );

    // The host's player is the host's to move; the client can't push it.
    assert!(host.0.is_owner(mine));
    host.0.set_position(mine, Vector3::new(3.0, 0.0, 0.0));
    pump(&mut host, &mut client, 40);
    let mine_remote = players(&client.0)
        .into_iter()
        .find(|(_, n)| n.owner == HOST_CLIENT_ID)
        .unwrap()
        .0;
    assert!((client.0.position(mine_remote) - Vector3::new(3.0, 0.0, 0.0)).length() < 1e-3);

    // A client that joins late receives everything already spawned.
    let port = host.0.network().port;
    let mut late = peer(&dir);
    late.0
        .network_mut()
        .start_client(&format!("127.0.0.1:{port}"))
        .unwrap();
    for _ in 0..200 {
        pump(&mut host, &mut late, 1);
        if players(&late.0).len() == 2 {
            break;
        }
    }
    assert_eq!(players(&late.0).len(), 2);
}

#[test]
fn messages_round_trip() {
    let m = Msg::Rpc {
        id: 7,
        name: "Hit".into(),
        arg: Value::Float(1.5),
    };
    match decode(&encode(&m)) {
        Some(Msg::Rpc { id, name, arg }) => {
            assert_eq!((id, name.as_str()), (7, "Hit"));
            assert!(matches!(arg, Value::Float(f) if f == 1.5));
        }
        other => panic!("{other:?}"),
    }
}
