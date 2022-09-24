//! Multiplayer webRTC server test with Bevy
use std::any::TypeId;
use std::io::Write;
use std::net::TcpListener;

use async_tungstenite::accept_hdr_async;
use async_tungstenite::tungstenite::handshake::client::Request;
use async_tungstenite::tungstenite::handshake::server::{ErrorResponse, Response};
use async_tungstenite::tungstenite::http::HeaderValue;
use async_tungstenite::tungstenite::Message;
use bevy::app::{PluginGroupBuilder, ScheduleRunnerPlugin};
use bevy::asset::AssetPlugin;
use bevy::audio::AudioPlugin;
use bevy::core::CorePlugin;
use bevy::core_pipeline::CorePipelinePlugin;
use bevy::diagnostic::DiagnosticsPlugin;
use bevy::ecs::query::WorldQuery;
use bevy::gltf::GltfPlugin;
use bevy::input::InputPlugin;
use bevy::log::{LogPlugin, LogSettings};
use bevy::pbr::PbrPlugin;
use bevy::reflect::serde::ReflectSerializer;
use bevy::reflect::{FromType, TypeRegistry};
use bevy::render::RenderPlugin;
use bevy::scene::ScenePlugin;
use bevy::sprite::SpritePlugin;
use bevy::tasks::{IoTaskPool, TaskPoolBuilder};
use bevy::text::TextPlugin;
use bevy::time::TimePlugin;
use bevy::ui::UiPlugin;
use bevy::utils::HashMap;
use bevy::window::WindowPlugin;
use bevy::winit::WinitPlugin;
use bevy::{app::ScheduleRunnerSettings, prelude::*, utils::Duration};
use futures::prelude::*;
use futures_util::{StreamExt, TryStreamExt};
use messages::{PlayerMessage, ServerMessage};
use serde::Serialize;
use shared_components::{NSprite, NTransform};

type ServerMessageSenders = HashMap<u64, futures::channel::mpsc::Sender<messages::ServerMessage>>;
type PlayerMessageReceiver = futures::channel::mpsc::Receiver<(u64, messages::PlayerMessage)>;
type PlayerMessageSender = futures::channel::mpsc::Sender<(u64, messages::PlayerMessage)>;
type ConnectionMappings = HashMap<u64, messages::PlayerId>;

struct WsServer {}

struct ServerSettings {
    ip_address: String,
    channel_size: usize,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            ip_address: "127.0.0.1:13037".to_string(),
            channel_size: 1024,
        }
    }
}

struct NetworkPlugin;

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<Broadcast>()
            .add_event::<PlayerEvent>()
            .add_startup_system(start_websocket_server)
            .add_system(accept_websocket_connections)
            .add_system(pump_messages)
            .add_system_to_stage(CoreStage::PostUpdate, broadcast_messages);
    }
}

struct MyPlugins;

impl PluginGroup for MyPlugins {
    fn build(&mut self, group: &mut PluginGroupBuilder) {
        group.add(LogPlugin::default());
        group.add(CorePlugin::default());
        group.add(TimePlugin::default());
        group.add(TransformPlugin::default());

        group.add(AssetPlugin::default());
        group.add(ScheduleRunnerPlugin::default());
        group.add(NetworkPlugin);
    }
}

#[derive(Clone, Debug)]
enum PlayerEvent {
    PlayerJoined,
    PlayerLeft,
}

#[derive(Clone, Debug)]
enum Broadcast {
    ComponentChanged {
        entity: Entity,
        component: u16,
        data: Vec<u8>,
    },
    ComponentAdded {
        entity: Entity,
        component: u16,
        data: Vec<u8>,
    },
}

fn networked<T: Component + Serialize + Reflect>(
    query: Query<(Entity, &T, ChangeTrackers<T>)>,
    type_mappings: Res<(HashMap<u16, TypeId>, HashMap<TypeId, u16>)>,
    type_registry: Res<TypeRegistry>,
    mut broadcasts: ResMut<Events<Broadcast>>,
) {
    for (entity, component, tracker) in query.iter() {
        if tracker.is_added() || tracker.is_changed() {
            // let read_register = type_registry.read();
            // let reflect_serializer = ReflectSerializer::new(component, &*read_register);
            // let mut buffer = Vec::new();
            // let mut serializer = rmp_serde::Serializer::new(&mut buffer)
            //     .with_binary()
            //     .with_struct_tuple();
            //
            // reflect_serializer.serialize(&mut serializer).unwrap();
            // let data = buffer;
            // let data = rmp_serde::to_vec(&component).unwrap();
            let data = postcard::to_allocvec(&component).unwrap();

            broadcasts.send(Broadcast::ComponentChanged {
                entity,
                component: *type_mappings.1.get(&std::any::TypeId::of::<T>()).unwrap(),
                data,
            });
        }
    }
}

fn main() {
    let mut options = DefaultTaskPoolOptions::with_num_threads(16);
    App::new()
        .insert_resource(ScheduleRunnerSettings::run_loop(Duration::from_secs_f64(
            1.0 / 30.0,
        )))
        .insert_resource(shared_components::kind_to_type_id_mappings())
        .insert_resource(ServerSettings::default())
        .insert_resource(LogSettings {
            filter: "debug,wgpu=warn".into(),
            level: bevy::log::Level::DEBUG,
        })
        .insert_resource(options)
        .add_plugins(MyPlugins)
        .register_type::<shared_components::NTransform>()
        .add_system(networked::<shared_components::NTransform>)
        .register_type::<shared_components::NSprite>()
        .add_system(networked::<shared_components::NSprite>)
        .add_startup_system(spawn_npcs)
        .add_system(translate_transform)
        .add_system(move_entities)
        .add_system(animate_sprite)
        .add_system(counter)
        // .add_system(get_new_target)
        .run();
}

fn start_websocket_server(mut commands: Commands, settings: Res<ServerSettings>) {
    let server_message_senders = ServerMessageSenders::new();
    let (player_message_sender, player_message_receiver) =
        futures::channel::mpsc::channel::<(u64, messages::PlayerMessage)>(settings.channel_size);

    debug!("starting tcp listener at {}", &settings.ip_address);
    let server = TcpListener::bind(&settings.ip_address).expect("Failed to start server");
    server.set_nonblocking(true);

    let io_pool = IoTaskPool::get();
    io_pool
        .spawn(async {
            debug!("debug from future 1");
            println!("println from future 1");
            std::io::stdout().flush().unwrap();
            future::ok::<(), ()>(())
        })
        .detach();

    commands.insert_resource(server);
    commands.insert_resource(server_message_senders);
    commands.insert_resource(player_message_sender);
    commands.insert_resource(player_message_receiver);

    let io = IoTaskPool::get();
    debug!("io threads: {}", io.thread_num());
}

fn add_csp(_request: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    response.headers_mut().insert(
        "Content-Security-Policy",
        HeaderValue::from_str("connect-src self ws://127.0.0.1:13037/").unwrap(),
    );
    Ok(response)
}

fn accept_websocket_connections(
    server_settings: Res<ServerSettings>,
    server: Res<TcpListener>,
    mut server_message_senders: ResMut<ServerMessageSenders>,
    player_message_sender: Res<PlayerMessageSender>,
    mut next_connection_id: Local<u64>,
) {
    let io_pool = IoTaskPool::get();

    while let Ok((stream, addr)) = server.accept() {
        debug!("new connection from {:?}", addr);

        let connection_id = *next_connection_id;
        *next_connection_id += 1;

        let (server_message_sender, server_message_receiver) =
            futures::channel::mpsc::channel(server_settings.channel_size);
        let player_message_sender = player_message_sender.clone();

        server_message_senders.insert(connection_id, server_message_sender);

        debug!("spawning io task for connection {}", connection_id);

        io_pool.spawn(async move {
            let mut server_message_receiver= server_message_receiver;
            let mut player_message_sender = player_message_sender;
            println!("accepting websocket connection from {}", connection_id);
            let websocket = accept_hdr_async(async_std::net::TcpStream::from(stream), add_csp)
                .await
                .expect("Error during the websocket handshake occurred");
            let (mut ws_write, ws_read) = websocket.split();

            // wait for a hello message
            let mut filtered = ws_read
                .try_filter(|msg| future::ready(msg.is_binary()))
                .map_err(|_| ());

            println!("starting to poll messages from {}", connection_id);
            loop {
                // check if we have a message to send
                let mut next_message = filtered.next().fuse();
                let mut next_send = server_message_receiver.next().fuse();

                futures::select! {
                    send = next_send => {
                        // let data = rmp_serde::to_vec(&send).unwrap();
                        let data = postcard::to_allocvec(&send).unwrap();
                        debug!("encoded length: {}", data.len());
                        ws_write.send(Message::Binary(data[1..].to_vec())).await;
                    },
                    msg = next_message => {
                        if let Some(msg) = msg {
                            let data = msg.unwrap().into_data();
                            println!("data: {:02X?}", &data);
                            // let message: PlayerMessage =
                            //     rmp_serde::from_read_ref(&data).expect("failed to parse player message");
                            let message: PlayerMessage =
                                postcard::from_bytes(&data).expect("failed to parse player message");

                            match player_message_sender.try_send((connection_id, message)) {
                                Ok(_) => future::ok(()),
                                Err(_) => {
                                    debug!("failed to add player message to channel, exiting future");
                                    future::err(())
                                }
                            }.await;
                        } else {
                            debug!("connection {}: websocket returned None, exiting io task", connection_id);
                            break
                        }
                    }
                }
            }
        }).detach();
    }
}

fn pump_messages(mut receive: ResMut<PlayerMessageReceiver>) {
    while let Ok(Some((connection_id, message))) = receive.try_next() {
        println!("got a message from {}: {:?}", connection_id, message);
    }
}

fn broadcast_messages(
    mut senders: ResMut<ServerMessageSenders>,
    mut broadcasts: EventReader<Broadcast>,
) {
    for b in broadcasts.iter() {
        let b: &Broadcast = b;

        for (conn_id, sender) in senders.iter_mut() {
            let sender: &mut futures::channel::mpsc::Sender<ServerMessage> = sender;
            let msg = match b {
                Broadcast::ComponentChanged {
                    entity,
                    component,
                    data,
                } => ServerMessage::ComponentChanged {
                    entity: entity.into(),
                    component: component.clone(),
                    data: data.clone(),
                },
                Broadcast::ComponentAdded {
                    entity,
                    component,
                    data,
                } => ServerMessage::ComponentAdded {
                    entity: entity.into(),
                    component: component.clone(),
                    data: data.clone(),
                },
            };
            sender
                .try_send(msg)
                .expect("failed to send broadcast message");
        }
    }
}

#[derive(Component)]
struct Npc;

#[derive(Component, Deref, DerefMut)]
struct AnimationTimer(Timer);
#[derive(Component)]
struct MoveTarget(Vec3);

fn spawn_npcs(mut commnads: Commands) {
    for _ in 0..50 {
        let random_x = rand::random::<f32>() * 1200. - 600.;
        let random_y = rand::random::<f32>() * 600. - 300.;

        commnads
            .spawn_bundle(TransformBundle::default())
            .insert(NSprite::default())
            .insert(NTransform::default())
            .insert(Npc)
            .insert(AnimationTimer(Timer::from_seconds(0.1, true)))
            .insert(MoveTarget(Vec3::new(random_x, random_y, 0.)));
    }
}

fn translate_transform(mut entities: Query<(&Transform, &mut NTransform), Changed<Transform>>) {
    for (t, mut nt) in entities.iter_mut() {
        *nt = NTransform::from(t.clone())
    }
}

fn animate_sprite(time: Res<Time>, mut query: Query<(&mut AnimationTimer, &mut NSprite)>) {
    for (mut timer, mut sprite) in &mut query {
        timer.tick(time.delta());
        if timer.just_finished() {
            sprite.sprite_index = (sprite.sprite_index + 1) % 4;
        }
    }
}

fn counter(mut state: Local<CounterState>) {
    if state.count % 60 == 0 {
        println!("{}", state.count);
    }
    state.count += 1;
}

#[derive(Default)]
struct CounterState {
    count: u32,
}

fn move_entities(
    mut entities: Query<(Entity, &mut Transform, &MoveTarget)>,
    mut commands: Commands,
) {
    for (entity, mut transform, target) in entities.iter_mut() {
        let distance = transform.translation.distance(target.0).min(1.0);
        let direction = (target.0 - transform.translation).normalize_or_zero();
        transform.translation += direction * distance;

        if distance <= 0.01 {
            commands.entity(entity).remove::<MoveTarget>();
            println!("entity reached target!");
        }
    }
}

fn get_new_target(mut commands: Commands, entities: Query<(Entity,), Without<MoveTarget>>) {
    for (entity,) in entities.iter() {
        let random_x = rand::random::<f32>() * 1200. - 600.;
        let random_y = rand::random::<f32>() * 600. - 300.;

        commands
            .entity(entity)
            .insert(MoveTarget(Vec3::new(random_x, random_y, 0.)));
    }
}
