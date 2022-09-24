//! Multiplayer webRTC test with Bevy

use bevy::app::PluginGroupBuilder;
use bevy::ecs::system::SystemState;
use bevy::log::LogSettings;
use bevy::reflect::erased_serde::__private::serde::de::DeserializeSeed;
use bevy::reflect::serde::ReflectDeserializer;
use bevy::reflect::TypeRegistry;
use bevy::render::camera::RenderTarget;
use bevy::tasks::IoTaskPool;
use bevy::utils::HashMap;
use bevy::{prelude::*, render::texture::ImageSettings};
use futures::channel::mpsc::Receiver;
use futures::prelude::*;
use messages::{KindId, NetworkEntity, ServerMessage};
use shared_components::{NSprite, NTransform};
use std::any::TypeId;
use ws_stream_wasm::*;

#[derive(Component)]
struct MoveTarget(Vec3);
#[derive(Component)]
struct PlayerControlled;

fn main() {
    // When building for WASM, print panics to the browser console
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    App::new()
        .register_type::<NSprite>()
        .register_type::<NTransform>()
        .insert_resource(WindowDescriptor {
            present_mode: bevy::window::PresentMode::AutoVsync,
            ..Default::default()
        })
        .insert_resource(shared_components::kind_to_type_id_mappings())
        .insert_resource(LogSettings {
            filter: "warn,client=error".into(),
            level: bevy::log::Level::DEBUG,
        })
        .insert_resource(ImageSettings::default_nearest()) // prevents blurry sprites
        .add_plugins(DefaultPlugins)
        .add_startup_system(setup)
        .add_startup_system(spawn_websocket_client)
        .add_system(handle_server_message)
        .add_system(translate_sprites)
        .add_system(translate_transfom)
        // .add_system(animate_sprite)
        // .add_system(player_input)
        // .add_system(move_entities)
        .run();
}

fn handle_server_message(
    mut commands: Commands,
    entity_finder: Query<(Entity, &NetworkEntity)>,
    type_mappings: Res<(HashMap<u16, TypeId>, HashMap<TypeId, u16>)>,
    mut receiver: ResMut<Receiver<ServerMessage>>,
    mut entity_lookup: Local<HashMap<NetworkEntity, Entity>>,
) {
    let mut find_entity = |network_entity: &NetworkEntity, commands: &mut Commands| {
        if let Some(entity) = entity_lookup.get(network_entity) {
            entity.clone()
        } else {
            let entity = if let Some((entity, _)) =
                entity_finder.iter().find(|(_, ne)| **ne == *network_entity)
            {
                entity.clone()
            } else {
                debug!("spawned a new entity: {:?}", &network_entity);
                commands.spawn_bundle((*network_entity,)).id()
            };
            entity_lookup.insert(*network_entity, entity);
            entity
        }
    };

    while let Ok(Some(msg)) = receiver.try_next() {
        match msg {
            ServerMessage::Welcome { .. } => {}
            ServerMessage::Refresh { .. } => {}
            ServerMessage::ComponentAdded { .. } => {}
            ServerMessage::ComponentChanged {
                entity,
                component,
                data,
            } => {
                let e = find_entity(&entity, &mut commands);
                let type_id = type_mappings.0.get(&component).unwrap().clone();

                commands.add(move |world: &mut World| {
                    world.resource_scope(|world, register: Mut<TypeRegistry>| {
                        let read_registry = register.read();
                        //let deser = ReflectDeserializer::new(&*read_registry);

                        let registration = read_registry
                            .get(type_id)
                            .expect("invalid component received");

                        let deser = registration.data::<ReflectDeserialize>().unwrap();

                        // let mut deserializer = rmp_serde::Deserializer::from_read_ref(&data);
                        let mut deserializer = postcard::Deserializer::from_bytes(&data);

                        let component_de = deser
                            .deserialize(&mut deserializer)
                            .expect("failed to deserialize component");

                        registration.data::<ReflectComponent>().unwrap().insert(
                            world,
                            e,
                            component_de.as_ref(),
                        )
                    });
                });
            }
        }
    }
}

fn translate_sprites(
    mut commands: Commands,
    mut nsprites: Query<(Entity, &NSprite, Option<&mut TextureAtlasSprite>)>,
    game_assets: Res<GameAssets>,
) {
    for (entity, n_sprite, mut sprite) in nsprites.iter_mut() {
        if let Some(mut sprite) = sprite {
            sprite.index = n_sprite.sprite_index as usize;
        } else {
            debug!("tetsingasf");
            commands.entity(entity).insert_bundle(SpriteSheetBundle {
                sprite: TextureAtlasSprite::new(n_sprite.sprite_index as usize),
                texture_atlas: game_assets.sprite_atlas.clone(),
                ..Default::default()
            });
        }
    }
}

fn translate_transfom(
    mut commands: Commands,
    mut entities: Query<(
        Entity,
        &NTransform,
        Option<&mut Transform>,
        Option<&TextureAtlasSprite>,
    )>,
) {
    for (entity, nt, maybe_transform, s) in entities.iter_mut() {
        if let Some(mut transform) = maybe_transform {
            let new_transform = nt.as_transform();
            transform.translation = new_transform.translation;
            debug!("new translation: {:?}", &transform.translation);
            debug!("entity {:?} has sprite: {}", &entity, s.is_some())
            //*transform = nt.as_transform();
        } else {
            commands.entity(entity).insert(nt.as_transform());
            debug!("entity had not transform");
        }
    }
}

fn spawn_websocket_client(mut commands: Commands) {
    let (mut player_message_sender, mut player_message_receiver) =
        futures::channel::mpsc::channel::<messages::PlayerMessage>(512);
    let (mut server_message_sender, server_message_receiver) =
        futures::channel::mpsc::channel::<messages::ServerMessage>(512);
    let player_id = messages::PlayerId::new();

    let io_pool = IoTaskPool::get();

    debug!("spawning ws task");
    io_pool
        .spawn(async move {
            debug!("connecting to server");
            let (_ws_meta, ws_stream) = WsMeta::connect("ws://127.0.0.1:13037/", None)
                .await
                .unwrap();
            debug!("connected to server");

            let (mut ws_write, mut ws_read) = ws_stream.split();

            loop {
                // see if we want to send anything
                let mut pending_send = player_message_receiver.next().fuse();
                // receive from server
                let mut next_message = ws_read.next().fuse();
                futures::select! {
                    msg = pending_send => {
                        ws_write
                            // .send(WsMessage::Binary(rmp_serde::to_vec(&msg).unwrap()))
                            .send(WsMessage::Binary(postcard::to_allocvec(&msg).unwrap()))
                            .await
                            .unwrap();
                    }
                    msg = next_message => {
                        if let Some(msg) = msg {
                            let data: Vec<u8> = match msg {
                                WsMessage::Binary(data) => data,
                                _ => panic!("text message")
                            };
                            // let message: messages::ServerMessage = rmp_serde::from_read_ref(&data[..]).expect("failed to parse server message");
                            let message: messages::ServerMessage = postcard::from_bytes(&data[..]).expect("failed to parse server message");
                            server_message_sender.send(message).await.unwrap();
                        } else {
                            error!("got none, socket disconnected?");
                            break;
                        }
                    }
                }
            }
        })
        .detach();

    commands.insert_resource(player_message_sender);
    commands.insert_resource(server_message_receiver);
    commands.insert_resource(player_id);
}

struct GameAssets {
    sprite_atlas: Handle<TextureAtlas>,
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut texture_atlases: ResMut<Assets<TextureAtlas>>,
) {
    let texture_handle = asset_server.load("creature-sheet.png");
    let texture_atlas = TextureAtlas::from_grid(texture_handle, Vec2::new(48.0, 48.0), 4, 1);
    let texture_atlas_handle = texture_atlases.add(texture_atlas);

    commands.insert_resource(GameAssets {
        sprite_atlas: texture_atlas_handle,
    });

    commands.spawn_bundle(Camera2dBundle::default());
}

fn player_input(
    mut commands: Commands,
    windows: Res<Windows>,
    buttons: Res<Input<MouseButton>>,
    q_camera: Query<(&Camera, &GlobalTransform)>,
    mut q_player: Query<(Entity, Option<&mut MoveTarget>), With<PlayerControlled>>,
) {
    // get the camera info and transform
    // assuming there is exactly one main camera entity, so query::single() is OK
    let (camera, camera_transform) = q_camera.single();

    // get the window that the camera is displaying to (or the primary window)
    let wnd = if let RenderTarget::Window(id) = camera.target {
        windows.get(id).unwrap()
    } else {
        windows.get_primary().unwrap()
    };

    // check if the cursor is inside the window and get its position
    if let Some(screen_pos) = wnd.cursor_position() {
        if buttons.just_pressed(MouseButton::Left) {
            // get the size of the window
            let window_size = Vec2::new(wnd.width() as f32, wnd.height() as f32);

            // convert screen position [0..resolution] to ndc [-1..1] (gpu coordinates)
            let ndc = (screen_pos / window_size) * 2.0 - Vec2::ONE;

            // matrix for undoing the projection and camera transform
            let ndc_to_world =
                camera_transform.compute_matrix() * camera.projection_matrix().inverse();

            // use it to convert ndc to world-space coordinates
            let mut world_pos = ndc_to_world.project_point3(ndc.extend(-1.0));

            // reduce it to a 2D value
            world_pos.z = 0.;
        }
    }
}
