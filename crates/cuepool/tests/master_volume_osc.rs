//! Exercise the production OSC decoder/router, application volume queue and
//! reply transport together, without opening audio hardware or a window.
use cuepool::master_volume;
use cuepool_core::{ShowFile, showfile::parse_show_file};
use cuepool_gui::{SharedState, SharedStateHandle};
use cuepool_protocols::osc::{OscEvent, OscManager};
use rosc::{OscMessage, OscPacket, OscType};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

struct Rig {
    manager: OscManager,
    events: mpsc::Receiver<OscEvent>,
    destination: SocketAddr,
    outbound: UdpSocket,
    state: SharedStateHandle,
    applied: Vec<f32>,
}

fn socket() -> UdpSocket {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
}

fn send(socket: &UdpSocket, destination: SocketAddr, addr: &str, args: Vec<OscType>) {
    let bytes = rosc::encoder::encode(&OscPacket::Message(OscMessage {
        addr: addr.into(),
        args,
    }))
    .unwrap();
    socket.send_to(&bytes, destination).unwrap();
}

fn receive(socket: &UdpSocket) -> (i32, f32) {
    let mut buf = [0; 256];
    let len = socket.recv(&mut buf).unwrap();
    let (_, OscPacket::Message(message)) = rosc::decoder::decode_udp(&buf[..len]).unwrap() else {
        panic!("expected a single OSC message");
    };
    assert_eq!(message.addr, "/qplayer/volume/state");
    let [OscType::Int(id), OscType::Float(db)] = message.args.as_slice() else {
        panic!("state must have exactly int32, float32 arguments");
    };
    (*id, *db)
}

fn assert_quiet(socket: &UdpSocket) {
    socket.set_nonblocking(true).unwrap();
    assert_eq!(
        socket.recv(&mut [0; 256]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    socket.set_nonblocking(false).unwrap();
}

impl Rig {
    fn new() -> Self {
        let probe = socket();
        let destination = probe.local_addr().unwrap();
        drop(probe);
        let outbound = socket();
        let (tx, events) = mpsc::channel();
        let manager = OscManager::new(
            Ipv4Addr::LOCALHOST,
            destination.port(),
            outbound.local_addr().unwrap().port(),
            tx,
        )
        .unwrap();
        Self {
            manager,
            events,
            destination,
            outbound,
            state: Arc::new(Mutex::new(SharedState::default())),
            applied: Vec::new(),
        }
    }

    fn enqueue(&self, count: usize) {
        for _ in 0..count {
            let event = self.events.recv_timeout(Duration::from_secs(2)).unwrap();
            master_volume::enqueue(&self.state, event).unwrap();
        }
    }

    fn drain(&mut self) {
        let commands = std::mem::take(&mut self.state.lock().unwrap().command_queue);
        for command in commands {
            let reply = master_volume::process(&self.state, command, || {
                self.applied.push(
                    self.state
                        .lock()
                        .unwrap()
                        .show_file
                        .show_settings
                        .master_volume_db,
                );
            })
            .unwrap();
            if let Some((destination, message)) = reply {
                self.manager.send_to(message, destination).unwrap();
            }
        }
    }
}

#[test]
fn commands_and_queries_reply_in_processed_order_to_the_requesting_socket() {
    let mut rig = Rig::new();
    let a = socket();
    let b = socket();
    send(
        &a,
        rig.destination,
        "/qplayer/volume/get",
        vec![OscType::Int(i32::MIN)],
    );
    send(
        &a,
        rig.destination,
        "/qplayer/volume",
        vec![OscType::Float(-6.0)],
    );
    send(
        &a,
        rig.destination,
        "/qplayer/volume/get",
        vec![OscType::Int(0)],
    );
    rig.enqueue(3);
    send(
        &b,
        rig.destination,
        "/qplayer/volume",
        vec![OscType::Float(-12.0), OscType::Int(i32::MAX)],
    );
    rig.enqueue(1);
    send(
        &a,
        rig.destination,
        "/qplayer/volume/get",
        vec![OscType::Int(27)],
    );
    rig.enqueue(1);
    // The RX thread must neither mutate the setting nor answer before dispatch.
    assert_eq!(
        rig.state
            .lock()
            .unwrap()
            .show_file
            .show_settings
            .master_volume_db,
        0.0
    );
    assert_quiet(&a);
    assert_quiet(&b);
    rig.drain();
    assert_eq!(receive(&a), (i32::MIN, 0.0));
    assert_eq!(receive(&a), (0, -6.0));
    assert_eq!(receive(&b), (i32::MAX, -12.0));
    assert_eq!(receive(&a), (27, -12.0));
    assert_eq!(rig.applied, [-6.0, -12.0]);
    assert_quiet(&a);
    assert_quiet(&b);
    assert_quiet(&rig.outbound);
}

#[test]
fn confirmation_returns_clamped_gain_including_silence_and_unchanged_sets() {
    let mut rig = Rig::new();
    let client = socket();
    for (id, (input, expected)) in [
        (-120.0, -96.0),
        (-96.0, -96.0),
        (20.0, 12.0),
        (f32::NEG_INFINITY, -96.0),
        (f32::INFINITY, 12.0),
        (f32::NAN, 0.0),
    ]
    .into_iter()
    .enumerate()
    {
        send(
            &client,
            rig.destination,
            "/qplayer/volume",
            vec![OscType::Float(input), OscType::Int(id as i32)],
        );
        rig.enqueue(1);
        rig.drain();
        assert_eq!(receive(&client), (id as i32, expected));
        assert_eq!(
            rig.state
                .lock()
                .unwrap()
                .show_file
                .show_settings
                .master_volume_db,
            expected
        );
    }
    assert_eq!(rig.applied, [-96.0, 12.0, -96.0, 12.0, 0.0]);
    assert!(rig.state.lock().unwrap().dirty);
}

#[test]
fn legacy_numeric_setters_keep_working_without_unsolicited_replies() {
    let mut rig = Rig::new();
    let client = socket();
    for level in [
        OscType::Float(-6.0),
        OscType::Int(-7),
        OscType::Double(-8.5),
        OscType::Long(-9),
    ] {
        send(&client, rig.destination, "/qplayer/volume", vec![level]);
    }
    rig.enqueue(4);
    rig.drain();
    assert_eq!(rig.applied, [-6.0, -7.0, -8.5, -9.0]);
    assert_quiet(&client);
    assert_quiet(&rig.outbound);
}

#[test]
fn malformed_requests_do_not_mutate_or_reply() {
    let mut rig = Rig::new();
    let client = socket();
    for args in [
        vec![],
        vec![OscType::Float(1.0)],
        vec![OscType::Long(1)],
        vec![OscType::String("1".into())],
        vec![OscType::Int(1), OscType::Int(2)],
    ] {
        send(&client, rig.destination, "/qplayer/volume/get", args);
    }
    for args in [
        vec![],
        vec![OscType::String("-6".into())],
        vec![OscType::Bool(true)],
        vec![OscType::Float(-6.0), OscType::Float(1.0)],
        vec![OscType::Float(-6.0), OscType::Int(1), OscType::Int(2)],
    ] {
        send(&client, rig.destination, "/qplayer/volume", args);
    }
    // A valid query is a barrier on this socket's ordered RX stream. Only it
    // should reach the application, rather than relying on an arbitrary sleep.
    send(
        &client,
        rig.destination,
        "/qplayer/volume/get",
        vec![OscType::Int(42)],
    );
    rig.enqueue(1);
    rig.drain();
    assert_eq!(receive(&client), (42, 0.0));
    assert!(rig.events.try_recv().is_err());
    assert!(rig.applied.is_empty());
    assert!(!rig.state.lock().unwrap().dirty);
    assert_quiet(&client);
}

#[test]
fn queries_read_the_show_setting_after_gui_edits_and_show_replacement() {
    let mut rig = Rig::new();
    let client = socket();
    // Queue first, then edit the same setting the Project Settings fader owns.
    send(
        &client,
        rig.destination,
        "/qplayer/volume/get",
        vec![OscType::Int(1)],
    );
    rig.enqueue(1);
    rig.state
        .lock()
        .unwrap()
        .show_file
        .show_settings
        .master_volume_db = -18.0;
    rig.drain();
    assert_eq!(receive(&client), (1, -18.0));

    let mut show = ShowFile::default();
    show.show_settings.master_volume_db = -32.0;
    let document = serde_json::to_string(&show).unwrap();
    rig.state.lock().unwrap().show_file = parse_show_file(&document).unwrap();
    send(
        &client,
        rig.destination,
        "/qplayer/volume/get",
        vec![OscType::Int(2)],
    );
    rig.enqueue(1);
    rig.drain();
    assert_eq!(receive(&client), (2, -32.0));
    // Readback also clamps a manually edited show, without dirtying it, and
    // never depends on the existence or lifetime of an audio device.
    rig.state
        .lock()
        .unwrap()
        .show_file
        .show_settings
        .master_volume_db = 100.0;
    send(
        &client,
        rig.destination,
        "/qplayer/volume/get",
        vec![OscType::Int(3)],
    );
    rig.enqueue(1);
    rig.drain();
    assert_eq!(receive(&client), (3, 12.0));
    assert!(!rig.state.lock().unwrap().dirty);
    assert!(rig.applied.is_empty());
}

#[test]
#[cfg(feature = "test-harness")]
fn engine_replacement_reapplies_the_same_persisted_level_that_queries_report() {
    let mut rig = Rig::new();
    let client = socket();
    let audio = cuepool_audio::AudioEngine::new_headless(8, 48_000);
    send(
        &client,
        rig.destination,
        "/qplayer/volume",
        vec![OscType::Float(-96.0), OscType::Int(1)],
    );
    rig.enqueue(1);
    let commands = std::mem::take(&mut rig.state.lock().unwrap().command_queue);
    for command in commands {
        let reply = master_volume::process(&rig.state, command, || {
            master_volume::apply_to_engine(&rig.state, &audio);
        })
        .unwrap();
        if let Some((destination, message)) = reply {
            // The silence gain has already reached the engine before its ack.
            assert_eq!(audio.mixer().master_volume(), 0.0);
            rig.manager.send_to(message, destination).unwrap();
        }
    }
    assert_eq!(receive(&client), (1, -96.0));
    let saved = serde_json::to_string(&rig.state.lock().unwrap().show_file).unwrap();
    drop(audio);
    rig.state.lock().unwrap().show_file = parse_show_file(&saved).unwrap();
    let replacement = cuepool_audio::AudioEngine::new_headless(2, 44_100);
    assert_eq!(replacement.mixer().master_volume(), 1.0);
    master_volume::apply_to_engine(&rig.state, &replacement);
    assert_eq!(replacement.mixer().master_volume(), 0.0);
    send(
        &client,
        rig.destination,
        "/qplayer/volume/get",
        vec![OscType::Int(2)],
    );
    rig.enqueue(1);
    rig.drain();
    assert_eq!(receive(&client), (2, -96.0));
}
