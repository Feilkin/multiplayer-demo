//! bevy sprite animation system

use bevy::asset::{AssetLoader, AssetPath, BoxedFuture, Error, LoadContext, LoadedAsset};
use bevy::prelude::*;
use bevy::reflect::TypeUuid;
use bevy::sprite::Rect;
use bevy::utils::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, TypeUuid)]
#[uuid = "dfcaf828-ebd1-4bc1-acb4-ab14db715331"]
pub struct AnimatorArchetype {
    animations: Arc<HashMap<String, Animation>>,
    frame_times: Arc<Vec<Duration>>,
    texture_atlas_handle: Option<Handle<TextureAtlas>>,
}

impl AnimatorArchetype {
    pub fn from_aseprite(data: &aseprite::SpritesheetData) -> Self {
        let frame_tags = data
            .meta
            .frame_tags
            .as_ref()
            .expect("missing frameTags from spritesheet");

        let (loops, others): (Vec<_>, Vec<_>) =
            frame_tags.iter().partition(|tag| &tag.name == "loop");

        let mut animations = HashMap::with_capacity(others.len());

        for animation in others {
            // find loop for this animation
            let (loop_start, loop_end) = loops
                .iter()
                .find(|l| l.from >= animation.from && l.to <= animation.to)
                .map(|l| (l.from, l.to))
                .unwrap_or((animation.from, animation.to));

            animations.insert(
                animation.name.clone(),
                Animation {
                    start_index: animation.from as usize,
                    end_index: animation.to as usize,
                    loop_start: loop_start as usize,
                    loop_end: loop_end as usize,
                },
            );
        }

        let frame_times = data
            .frames
            .iter()
            .map(|frame| Duration::from_millis(frame.duration as u64))
            .collect();

        AnimatorArchetype {
            animations: Arc::new(animations),
            frame_times: Arc::new(frame_times),
            texture_atlas_handle: None,
        }
    }

    pub fn set_texture_handle(&mut self, texture_atlas_handle: Handle<TextureAtlas>) {
        self.texture_atlas_handle = Some(texture_atlas_handle);
    }

    pub fn new_instance(&self) -> Animator {
        let idle = self
            .animations
            .get("idle")
            .expect("animation set should have idle animation")
            .clone();
        Animator {
            animations: Arc::clone(&self.animations),
            frame_times: Arc::clone(&self.frame_times),
            current_frame: idle.start_index,
            animation_timer: Timer::new(self.frame_times[idle.start_index].clone(), false),
            current_animation: idle,
            next_animation: None,
        }
    }

    pub fn texture_handle(&self) -> Option<&Handle<TextureAtlas>> {
        self.texture_atlas_handle.as_ref()
    }
}

#[derive(Default)]
struct AsepriteAnimatorLoader;

impl AssetLoader for AsepriteAnimatorLoader {
    fn load<'a>(
        &'a self,
        bytes: &'a [u8],
        load_context: &'a mut LoadContext,
    ) -> BoxedFuture<'a, anyhow::Result<(), Error>> {
        Box::pin(async move {
            let data: aseprite::SpritesheetData = serde_json::from_slice(bytes)?;
            let mut archetype = AnimatorArchetype::from_aseprite(&data);
            debug!("loaded animator from aseprite json");

            let maybe_texture = if let Some(img_path) = data.meta.image {
                debug!("found image from aseprite json");
                let img_path: AssetPath = AssetPath::from(&img_path).to_owned();
                let texture_handle = load_context.get_handle(img_path.clone());
                let mut texture_atlas = TextureAtlas::new_empty(
                    texture_handle,
                    Vec2::new(data.meta.size.w as f32, data.meta.size.h as f32),
                );

                // add frames to texture atlas
                for frame in &data.frames {
                    let orig = Vec2::new(frame.frame.x as f32, frame.frame.y as f32);
                    let size = Vec2::new(frame.frame.w as f32, frame.frame.h as f32);

                    texture_atlas.add_texture(Rect {
                        min: orig,
                        max: orig + size,
                    });
                }

                let texture_atlas_handle = load_context.set_labeled_asset(
                    "texture_atlas",
                    LoadedAsset::new(texture_atlas).with_dependency(img_path.clone()),
                );
                archetype.texture_atlas_handle = Some(texture_atlas_handle);

                Some(img_path)
            } else {
                None
            };

            let mut asset = LoadedAsset::new(archetype);

            if let Some(path) = maybe_texture {
                asset.add_dependency(path);
            }

            load_context.set_default_asset(asset);
            Ok(())
        })
    }

    fn extensions(&self) -> &[&str] {
        &["aseprite.json"]
    }
}

pub fn assign_animator(
    mut commands: Commands,
    unassigned: Query<(
        Entity,
        &Handle<AnimatorArchetype>,
        Option<&Transform>,
        Option<&GlobalTransform>,
    )>,
    archetypes: Res<Assets<AnimatorArchetype>>,
) {
    for (entity, handle, transform, global_transform) in unassigned.iter() {
        if let Some(archetype) = archetypes.get(&handle) {
            let mut entity_commands = commands.entity(entity);
            entity_commands.remove::<Handle<AnimatorArchetype>>();
            entity_commands.insert(archetype.new_instance());

            if let Some(texture_handle) = archetype.texture_atlas_handle.as_ref().cloned() {
                entity_commands.insert_bundle(SpriteSheetBundle {
                    sprite: Default::default(),
                    texture_atlas: texture_handle,
                    transform: transform.cloned().unwrap_or_default(),
                    global_transform: global_transform.cloned().unwrap_or_default(),
                    visibility: Default::default(),
                    computed_visibility: Default::default(),
                });
            }
        }
    }
}

pub fn animate_sprite(
    mut animators: Query<(&mut Animator, &mut TextureAtlasSprite)>,
    time: Res<Time>,
) {
    for (mut animator, mut sprite) in animators.iter_mut() {
        if animator.animation_timer.tick(time.delta()).finished() {
            animator.current_frame += 1;

            // if we have a next animation waiting, continue until end
            if animator.next_animation.is_some() {
                if animator.current_frame > animator.current_animation.end_index {
                    animator.current_animation = animator.next_animation.take().unwrap();
                    animator.current_frame = animator.current_animation.start_index;
                }
            } else {
                if animator.current_frame > animator.current_animation.loop_end {
                    animator.current_frame = animator.current_animation.loop_start;
                }
            }

            // reset the timer as it just fired
            let new_duration = animator.frame_times[animator.current_frame].clone();
            animator.animation_timer.set_duration(new_duration);
            animator.animation_timer.reset();

            // set the sprite
            sprite.index = animator.current_frame;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Animation {
    start_index: usize,
    end_index: usize,
    loop_start: usize,
    loop_end: usize,
}

#[derive(Default, Debug, Clone, Component)]
pub struct Animator {
    animations: Arc<HashMap<String, Animation>>,
    frame_times: Arc<Vec<Duration>>,
    current_animation: Animation,
    animation_timer: Timer,
    current_frame: usize,
    next_animation: Option<Animation>,
}

pub struct AnimatorPlugin;

impl Plugin for AnimatorPlugin {
    fn build(&self, app: &mut App) {
        app.add_asset::<AnimatorArchetype>()
            .init_asset_loader::<AsepriteAnimatorLoader>()
            .add_system(assign_animator)
            .add_system(animate_sprite);
    }
}
