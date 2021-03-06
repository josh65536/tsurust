use std::marker::PhantomData;
use std::fmt::Debug;
use std::hash::Hash;
use enum_dispatch::enum_dispatch;
use fnv::FnvHashMap;
use serde::{Deserialize, Serialize};

use crate::{board::{Board, Port, TLoc}, game_state::GameState, tile::{GAct, Kind, Tile}};
use crate::game_state::BaseGameState;
use crate::board::BaseBoard;
use crate::WrapBase;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GameId(pub u32);

#[enum_dispatch]
pub trait GenericGame {
    fn new_state(&self, num_players: u32) -> BaseGameState;

    fn board(&self) -> BaseBoard;
}

impl<G> GenericGame for G
where
    G: Game,
    BaseGameState: From<GameState<G>>,
    BaseBoard: From<G::Board>,
{
    fn new_state(&self, num_players: u32) -> BaseGameState {
        GameState::new(self, num_players).into()
    }

    fn board(&self) -> BaseBoard {
        self.board().clone().into()
    }
}

#[macro_export]
macro_rules! for_each_game {
    (internal ($dollar:tt) $path:ident $name:ident $ty:ident => $($body:tt)*) => {
        macro_rules! __mac {
            ($dollar(($dollar ($dollar $path:tt)*) :: $dollar $name:ident: $dollar $ty:ty,)*) => {$($body)*}
        }
        __mac! {
            ($crate::game::BaseGame)::Normal: $crate::game::PathGame<$crate::board::RectangleBoard, $crate::tile::RegularTile<4>>,
        }
    };

    ($path:ident::$name:ident, $ty:ident => $($body:tt)*) => {
        $crate::for_each_game! {
            internal ($) $path $name $ty => $($body)*
        }
    };
}

for_each_game! {
    p::x, t =>
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub enum BaseGame {
        $($x($t)),*
    }

    impl BaseGame {
        pub fn new_state(&self, num_players: u32) -> BaseGameState {
            match self { $($($p)*::$x(s) => GameState::new(s, num_players).wrap_base()),* }
        }

        pub fn board(&self) -> BaseBoard {
            match self { $($($p)*::$x(s) => s.board().clone().wrap_base()),* }
        }
    }

    $($crate::impl_wrap_base!(BaseGame::$x($t)))*;
}

pub trait Game: Clone + Debug + Serialize {
    type TLoc: TLoc;
    type Port: Port;
    type Kind: Kind;
    type GAct: GAct;
    type TileConfig: Clone + Debug;
    type Board: Board<TLoc = Self::TLoc, Port = Self::Port, Kind = Self::Kind, TileConfig = Self::TileConfig>;
    type Tile: Tile<Kind = Self::Kind, GAct = Self::GAct, TileConfig = Self::TileConfig>;

    /// The game's board
    fn board(&self) -> &Self::Board;

    /// All the ports that players can start at
    fn start_ports(&self) -> Vec<Self::Port>;

    /// The set of tiles the game uses
    fn all_tiles(&self) -> Vec<Self::Tile> {
        Self::Tile::all(self.board().tile_config())
    }

    /// Tiles of some kind that a player starts with
    fn num_tiles_per_player(&self, kind: &Self::Kind) -> u32;
}

/// A definition for a path game
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathGame<B: Board, T> {
    #[serde(bound = "")]
    board: B,
    #[serde(bound = "")]
    start_ports: Vec<<B as Board>::Port>,
    #[serde(bound = "")]
    tiles_per_player: FnvHashMap<<B as Board>::Kind, u32>,
    phantom: PhantomData<T>,
}

impl<K, C, B, T> PathGame<B, T>
where
    K: Kind,
    C: Clone + Debug,
    B: Board<Kind = K, TileConfig = C>,
    T: Tile<Kind = K, TileConfig = C>
{
    pub fn new<I: IntoIterator<Item = (B::Kind, u32)>>(
        board: B, start_ports: Vec<<B as Board>::Port>, tiles_per_player: I) -> Self {
        Self {
            board,
            start_ports,
            tiles_per_player: tiles_per_player.into_iter().collect(),
            phantom: PhantomData,
        }
    }
}

impl<K, C, B, T> Game for PathGame<B, T>
where
    K: Kind,
    C: Clone + Debug,
    B: Board<Kind = K, TileConfig = C>,
    T: Tile<Kind = K, TileConfig = C>
{
    type TLoc = B::TLoc;
    type Port = B::Port;
    type Kind = B::Kind;
    type GAct = T::GAct;
    type TileConfig = B::TileConfig;
    type Board = B;
    type Tile = T;

    fn board(&self) -> &Self::Board {
        &self.board
    }

    fn start_ports(&self) -> Vec<Self::Port> {
        self.start_ports.clone()
    }

    fn num_tiles_per_player(&self, kind: &Self::Kind) -> u32 {
        self.tiles_per_player[kind]
    }
}