//! Common messages for networking

use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::scene::DynamicScene;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

pub trait KindId {
    const KIND_ID: u16;
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Hash)]
pub struct PlayerId(bevy::utils::Uuid);

impl PlayerId {
    pub fn new() -> Self {
        PlayerId(bevy::utils::Uuid::new_v4())
    }
}

impl Display for PlayerId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Component, Hash)]
pub struct NetworkEntity(u64);

impl From<&Entity> for NetworkEntity {
    fn from(e: &Entity) -> Self {
        NetworkEntity((e.generation() as u64) << 32 | (e.id() as u64))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlayerMessage {
    Hello { my_id: PlayerId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome {
        players: Vec<PlayerId>,
    },
    Refresh {
        /// world data as bytes, because we can't directly ser/de a DynamicScene
        world: Vec<u8>,
        players: Vec<PlayerId>,
    },
    ComponentAdded {
        entity: NetworkEntity,
        component: u16,
        data: Vec<u8>,
    },
    ComponentChanged {
        entity: NetworkEntity,
        component: u16,
        data: Vec<u8>,
    },
}
