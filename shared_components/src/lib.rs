//! Shared networked components
//!
//! All components should derive Reflect, Component, and Serialize + Deserialize

use bevy::math::Vec3Swizzles;
use bevy::prelude::*;
use bevy::utils::HashMap;
use messages::KindId;
use serde::{Deserialize, Serialize};
use std::any::TypeId;

macro_rules! networked {
    (@extract_ident $map1:ident $map2:ident) => {};
    (@extract_ident $map1:ident $map2:ident $id:expr => $c:ident { $($field:ident: $type:ty),* $(,)?} $($tail:tt)*) => {
        networked!(@insert_mappings $map1 $map2 $c);
        networked!(@extract_ident $map1 $map2 $($tail)*);
    };
    (@extract_ident $map1:ident $map2:ident $id:expr => $c:ident($($type:ty),* $(,)?) $($tail:tt)*) => {
        networked!(@insert_mappings $map1 $map2 $c);
        networked!(@extract_ident $map1 $map2 $($tail:tt)*);
    };
    (@insert_mappings $map1:ident $map2:ident $c:ident) => {
        $map1.insert($c::KIND_ID, std::any::TypeId::of::<$c>());
        $map2.insert(std::any::TypeId::of::<$c>(), $c::KIND_ID);
    };
    (@component) => {};
    // struct
    (@component $id:expr => $c:ident { $($field:ident: $type:ty),* $(,)?} $($tail:tt)*) => {
        #[derive(Component, Serialize, Deserialize, Default, Reflect)]
        #[reflect(Component, Serialize, Deserialize)]
        pub struct $c {
            $(pub $field: $type),*
        }

        networked!(@impl_kind_id $c $id);
        networked!(@component $($tail)*);
    };
    // tuple struct
    (@component $id:expr => $c:ident($($type:ty),* $(,)?) $($tail:tt)*) => {
        #[derive(Component, Serialize, Deserialize, Default, Reflect)]
        #[reflect(Component, Serialize, Deserialize)]
        pub struct $c ($($type),*);

        networked!(@impl_kind_id $c $id);
        networked!(@component $($tail)*);
    };
    (@impl_kind_id $c:ident $id:expr) => {
        impl KindId for $c {
            const KIND_ID: u16 = $id;
        }
    };
    ($($tail:tt)+) => {
        pub fn kind_to_type_id_mappings() -> (HashMap<u16, TypeId>, HashMap<TypeId, u16>) {
            let mut map_to_type_id = HashMap::new();
            let mut map_to_kind_id = HashMap::new();

            networked!(@extract_ident map_to_type_id map_to_kind_id $($tail)+);

            (map_to_type_id, map_to_kind_id)
        }

        networked!(@component $($tail)+);
    };
}

networked! {
    100 => NSprite {
        sprite_index: u32,
    }
    200 => NTransform {
        translation: Vec2,
        scale: Vec2
    }
}

impl From<Transform> for NTransform {
    fn from(t: Transform) -> Self {
        NTransform {
            translation: t.translation.xy(),
            scale: t.scale.xy(),
        }
    }
}

impl NTransform {
    pub fn as_transform(&self) -> Transform {
        Transform::from_translation(self.translation.extend(0.)).with_scale(self.scale.extend(0.))
    }
}
