use std::f64::consts::TAU;



use std::fmt::{Debug, Display};
use std::hash::Hash;
use common::{for_each_tile, nalgebra, nalgebra as na, GameInstance};

use common::math::{Pt2, Vec3f, Vec3u, pt2};
use common::nalgebra::vector;
use common::{board::{BaseBoard, BasePort, Board, RectangleBoard}, for_each_board, for_each_game, game::{BaseGame, Game, PathGame}, math::Vec2, tile::{RegularTile, Tile}};
use common::board::{BaseTLoc, Port, TLoc};
use common::tile::{BaseGAct, BaseTile, Kind};
use format_xml::{xml, spaced};

use itertools::{Itertools, chain, iproduct, izip};
use specs::prelude::*;
use wasm_bindgen::{JsCast};
use web_sys::{DomParser, Element, SupportedType, SvgElement, SvgMatrix};

use crate::ecs::{Collider, Model, TLocLabel, TileSlot, Transform, TileLabel, TileSelect, TileToPlace, GameInstanceLabel};
use crate::game::GameWorld;
use crate::{SVG_NS, document};

//fn create_svg_element<S: JsCast>(name: &str) -> S {
//    web_sys::window().unwrap().document().unwrap().create_element_ns(Some("http://www.w3.org/2000/svg"), name)
//        .expect("SVG element could not be created")
//        .dyn_into()
//        .expect("Wrong type specified")
//}

pub fn parse_elem(elem_str: &str) -> Element {
    let elem = DomParser::new().unwrap().parse_from_string(elem_str, SupportedType::ApplicationXml)
        .expect("Element could not be created");
    elem.document_element().expect("Element doesn't have an element")
}

pub fn parse_svg(svg_str: &str) -> SvgElement {
    let svg = DomParser::new().unwrap().parse_from_string(svg_str, SupportedType::ImageSvgXml)
        .expect("SVG could not be created");
    svg.document_element().expect("SVG doesn't have an element")
        .dyn_into().expect("SVG is not an SVG")
}

/// State of the client screen
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenState {
    Lobby,
    StatelessGame,
    Game
}

impl Display for ScreenState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lobby => write!(f, "lobby"),
            Self::StatelessGame => write!(f, "stateless-game"),
            Self::Game => write!(f, "game"),
        }
    }
}
    
pub fn set_screen_state(state: ScreenState) {
    document().get_element_by_id("screen").unwrap().set_attribute("state", &state.to_string()).unwrap();
}

pub fn set_username(username: &str) {
    let escaped = html_escape::encode_text(username);
    document().get_element_by_id("username_1").unwrap().set_inner_html(&escaped);
    document().get_element_by_id("username_2").unwrap().set_inner_html(&escaped);
}

/// A rectangle.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl Rect {
    /// From left, top, width, height
    pub fn from_ltwh(left: f32, top: f32, width: f32, height: f32) -> Self {
        Self { left, top, width, height }
    }

    /// From left, top, right, bottom
    pub fn from_ltrb(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self::from_ltwh(left, top, right - left, bottom - top)
    }

    /// Converts this to a viewBox value string
    pub fn to_viewbox_value(self) -> String {
        format!("{} {} {} {}", self.left, self.top, self.width, self.height)
    }
}

/// Renders a game instance as the html string for a selectable game in the lobby
pub fn render_game_instance(game: &GameInstance) -> String {
    let title = format!("{}. Normal", game.id().0);
    let board = game.game().board();
    let board_svg = board.render();
    let board_bb = board.bounding_box();
    let status = if let Some(state) = game.state() {
        if state.game_over() { "Game Over" } else { "Game Started" }
    } else { "Game Not Started" };
    let players = game.players().iter().map(|player| html_escape::encode_text(player)).join("; ");

    xml!(
        <div class="game-box">
            <div class="title">{ title }</div>
            <svg xmlns={SVG_NS} class="board" viewBox={board_bb.to_viewbox_value()}>{ board_svg }</svg>
            <div class="status">{ status }</div>
            <div class="players">"Players: "{ players }</div>
        </div>
    ).to_string()
}

/// Creates a entity corresponding to a game instance.
pub fn game_entity(game: GameInstance, world: &mut World, id_counter: &mut u64) -> Entity {
    let elem = parse_elem(&render_game_instance(&game));
    world.create_entity()
        .with(Model::new(
            &elem, -(game.id().0 as i32), &GameWorld::game_panel(), id_counter
        ))
        .with(Collider::new(&elem))
        .with(GameInstanceLabel(game))
        .build()
}

pub trait SvgMatrixExt {
    /// Transforms a position with this matrix
    fn transform(&self, position: Pt2) -> Pt2;
}

impl SvgMatrixExt for SvgMatrix {
    fn transform(&self, position: Pt2) -> Pt2 {
        pt2(
            self.a() as f64 * position.x + self.c() as f64 * position.y + self.e() as f64,
            self.b() as f64 * position.x + self.d() as f64 * position.y + self.f() as f64,
        )
    }
}


/// Extension trait for Board, mainly for rendering since
/// the server should know nothing about rendering
pub trait BoardExt: Board {
    /// Gets the bounding box of the board in SVG space
    fn bounding_box(&self) -> Rect;

    /// Render the tile to an SVG string. Returns a string instead of SvgElement for ease of use with `xml!`
    fn render(&self) -> String;

    fn port_position(&self, port: &Self::Port) -> Pt2;

    fn loc_position(&self, loc: &Self::TLoc) -> Pt2;

    /// Render the collider for a specific tile location.
    fn render_collider(&self, loc: &Self::TLoc) -> SvgElement;

    /// Creates an entity (mainly for collision detection) at a specific tile location.
    fn create_loc_collider_entity(&self, loc: &Self::TLoc, world: &mut World, id_counter: &mut u64) -> Entity;
}

impl BoardExt for RectangleBoard {
    fn bounding_box(&self) -> Rect {
        Rect::from_ltrb(-0.1, -0.1, self.width() as f32 + 0.1, self.height() as f32 + 0.1)
    }

    fn render(&self) -> String {
        format!(r##"<g xmlns="{}" class="rectangular-board">"##, SVG_NS) +
            &chain!(
                iproduct!(0..self.height(), 0..self.width()).map(|(y, x)|
                    xml!(<rect x={x} y={y} width="1" height="1"/>).to_string()),
                self.boundary_ports().into_iter().map(|(min, d)| {
                    let v = self.port_position(&(min, d));
                    let dx = if d.x == 0 { 0.1 } else { 0.0 };
                    let dy = if d.y == 0 { 0.1 } else { 0.0 };
                    xml!(<line x1={v.x - dx} x2={v.x + dx} y1={v.y - dy} y2={v.y + dy} class="rectangular-board-notch"/>).to_string()
                })
            )
                .join("") +
            r##"</g>"##
    }

    fn port_position(&self, port: &<Self as Board>::Port) -> Pt2 {
        port.0.cast::<f64>() + port.1.cast::<f64>() / (self.ports_per_edge() + 1) as f64
    }

    fn loc_position(&self, loc: &Self::TLoc) -> Pt2 {
        loc.cast() + vector![0.5, 0.5]
    }

    fn render_collider(&self, _loc: &Self::TLoc) -> SvgElement {
        let svg_str = xml! {
            <g xmlns={SVG_NS} fill="transparent">
                <rect x="-0.5" y="-0.5" width="1" height="1"/>
            </g>
        }.to_string();
        parse_svg(&svg_str)
    }

    fn create_loc_collider_entity(&self, loc: &Self::TLoc, world: &mut World, id_counter: &mut u64) -> Entity {
        let svg = self.render_collider(loc);
        world.create_entity()
            .with(Model::new(&svg, Collider::ORDER_TILE_LOC, &GameWorld::svg_root(), id_counter))
            .with(Collider::new(&svg))
            .with(Transform::new(self.loc_position(loc)))
            .with(TLocLabel(loc.wrap_base()))
            .with(TileSlot)
            .build()
    }
}

/// Extension trait for BaseBoard, mainly for rendering since
/// the server should know nothing about rendering
pub trait BaseBoardExt {
    fn bounding_box(&self) -> Rect;

    fn render(&self) -> String;
    
    fn port_position(&self, port: &BasePort) -> Pt2;

    fn loc_position(&self, loc: &BaseTLoc) -> Pt2;

    /// Creates an entity (mainly for collision detection) at a specific tile location.
    fn create_loc_collider_entity(&self, loc: &BaseTLoc, world: &mut World, id_counter: &mut u64) -> Entity;
}

for_each_board! {
    p::x, t => 

    impl BaseBoardExt for BaseBoard {
        fn bounding_box(&self) -> Rect {
            match self {
                $($($p)*::$x(b) => b.bounding_box()),*
            }
        }

        fn render(&self) -> String {
            match self {
                $($($p)*::$x(b) => b.render()),*
            }
        }

        fn port_position(&self, port: &BasePort) -> Pt2 {
            match self {
                $($($p)*::$x(b) => b.port_position(<$t as Board>::Port::unwrap_base_ref(port))),*
            }
        }

        fn loc_position(&self, loc: &BaseTLoc) -> Pt2 {
            match self {
                $($($p)*::$x(b) => b.loc_position(<$t as Board>::TLoc::unwrap_base_ref(loc))),*
            }
        }

        fn create_loc_collider_entity(&self, loc: &BaseTLoc, world: &mut World, id_counter: &mut u64) -> Entity {
            match self {
                $($($p)*::$x(b) => b.create_loc_collider_entity(
                    <$t as Board>::TLoc::unwrap_base_ref(loc),
                    world,
                    id_counter
                )),*
            }
        }
    }
}

/// Gets the point vectors of a `n`-sided regular polygon with unit side length,
/// centered at the origin, and rotated so there are 2 points with minimum y coordinate.
fn regular_polygon_points(n: u32) -> Vec<Vec2> {
    let radius = 0.5 / (TAU / (2.0 * n as f64)).sin();
    (0..n).map(|i| {
        let angle = TAU * (-0.25 + (-0.5 + i as f64) / n as f64);
        let (sin, cos) = angle.sin_cos();
        vector![cos * radius, sin * radius]
    }).collect_vec()
}

/// Gets the SVG string that draws a `n`-sided regular polygon with unit side length,
/// centered at the origin, and rotated so there are 2 points with minimum y coordinate.
fn regular_polygon_svg_str(n: u32) -> String {
    let poly_str = regular_polygon_points(n).into_iter()
        .map(|vec| format!("{},{}", vec.x, vec.y))
        .join(" ");
    xml!(<polygon points={poly_str}/>).to_string()
}

/// Extension trait for Tile, mainly for rendering since
/// the server should know nothing about rendering
pub trait TileExt: Tile {
    fn render(&self) -> String;
}

impl<const EDGES: u32> TileExt for RegularTile<EDGES> {
    fn render(&self) -> String {
        if self.visible() {
            let connections = (0..self.num_ports()).map(|i| self.output(i)).collect_vec();
            let _covered = vec![false; connections.len()];
            let poly_pts = regular_polygon_points(EDGES);
            let pts_normals = poly_pts.into_iter()
                .circular_tuple_windows()
                .flat_map(|(p0, p1)| {
                    let normal = vector![-p1.y + p0.y, p1.x - p0.x];
                    let ports_per_edge = self.ports_per_edge();
                    (0..ports_per_edge).map(move |i|
                        (p0 + (p1 - p0) * (i + 1) as f64 / (ports_per_edge + 1) as f64, normal)
                    )
                })
                .collect_vec();

            let curviness = 0.25;
            let path_str = izip!(0..self.num_ports(), connections)
                .map(|(s, t)| {
                    let p0 = pts_normals[s as usize].0;
                    let p1 = pts_normals[s as usize].0 + pts_normals[s as usize].1 * curviness;
                    let p2 = pts_normals[t as usize].0 + pts_normals[t as usize].1 * curviness;
                    let p3 = pts_normals[t as usize].0;
                    let result = xml!(
                        <path class="regular-tile-path-outer" d=("M "{p0.x}","{p0.y}" C "{p1.x}","{p1.y}" "{p2.x}","{p2.y}" "{p3.x}","{p3.y})/>
                        <path class="regular-tile-path-inner" d=("M "{p0.x}","{p0.y}" C "{p1.x}","{p1.y}" "{p2.x}","{p2.y}" "{p3.x}","{p3.y})/>
                    ).to_string();
                    result
                })
                .join("");

            let poly_str = regular_polygon_svg_str(EDGES);
            xml!(
                <g xmlns={SVG_NS} class="regular-tile-visible">{poly_str}{path_str}</g>
            ).to_string()
        } else {
            let poly_str = regular_polygon_svg_str(EDGES);
            xml!(
                <g xmlns={SVG_NS} class="regular-tile-hidden">{poly_str}</g>
            ).to_string()
        }
    }
}

/// Extension trait for BaseTile, mainly for rendering since
/// the server should know nothing about rendering
pub trait BaseTileExt {
    fn render(&self) -> String;

    fn create_hand_entity(&self, index: u32, action: &BaseGAct, world: &mut World, id_counter: &mut u64) -> Entity;

    fn create_board_entity_common<'a>(&self, world: &'a mut World, id_counter: &mut u64) -> EntityBuilder<'a>;

    fn create_to_place_entity(&self, action: &BaseGAct, transform: Transform, world: &mut World, id_counter: &mut u64) -> Entity;

    fn create_on_board_entity(&self, board: &BaseBoard, loc: &BaseTLoc, world: &mut World, id_counter: &mut u64) -> Entity;
}

for_each_tile! {
    p::x, t => 

    impl BaseTileExt for BaseTile {
        fn render(&self) -> String {
            match self { $($($p)*::$x(b) => b.render()),* }
        }

        fn create_hand_entity(&self, index: u32, action: &BaseGAct, world: &mut World, id_counter: &mut u64) -> Entity {
            match self { $($($p)*::$x(b) => {
                let svg = self.apply_action(action).render();
                let wrapper = parse_svg(&wrap_svg(&svg, ""));
                wrapper.set_attribute("class", "bottom-tile tile-unselected").expect("Cannot set tile select class");
                world.create_entity()
                    .with(TileLabel(self.clone()))
                    .with(Model::new(&wrapper, 0, &GameWorld::bottom_panel(), id_counter))
                    .with(Collider::new(&wrapper))
                    .with(TileSelect::new(self.kind(), index, action.clone()))
                    .build()
            }),* }
        }

        fn create_board_entity_common<'a>(&self, world: &'a mut World, id_counter: &mut u64) -> EntityBuilder<'a> {
            match self { $($($p)*::$x(b) => {
                world.create_entity()
                    .with(TileLabel(self.clone()))
            }),* }
        }

        fn create_to_place_entity(&self, action: &BaseGAct, transform: Transform, world: &mut World, id_counter: &mut u64) -> Entity {
            match self { $($($p)*::$x(b) => {
                let svg = self.apply_action(action).render();
                self.create_board_entity_common(world, id_counter)
                    .with(Model::new(&parse_svg(&svg), Model::ORDER_TILE_HOVER, &GameWorld::svg_root(), id_counter))
                    .with(TileToPlace)
                    .with(transform)
                    .build()
            }),* }
        }

        fn create_on_board_entity(&self, board: &BaseBoard, loc: &BaseTLoc, world: &mut World, id_counter: &mut u64) -> Entity {
            match self { $($($p)*::$x(b) => {
                let svg = self.render();
                self.create_board_entity_common(world, id_counter)
                    .with(Model::new(&parse_svg(&svg), Model::ORDER_TILE, &GameWorld::svg_root(), id_counter))
                    .with(Transform::new(board.loc_position(loc)))
                    .build()
            }),* }
        }
    }
}

/// Extension trait for Game, mainly for rendering since
/// the server should know nothing about rendering
pub trait GameExt: Game
where
    Self::Board: BoardExt
{
    /// Starting ports and their positions
    fn start_ports_and_positions(&self) -> Vec<(Self::Port, Pt2)> {
        self.start_ports().into_iter()
            .map(|port| (port.clone(), self.board().port_position(&port)))
            .collect()
    }
}

impl<K, C, B, T> GameExt for PathGame<B, T>
where
    K: Clone + Debug + Eq + Ord + Hash + Kind,
    C: Clone + Debug,
    B: Clone + Debug + Board<Kind = K, TileConfig = C> + BoardExt,
    T: Clone + Debug + Tile<Kind = K, TileConfig = C>
{}

/// Extension trait for BaseGame, mainly for rendering since
/// the server should know nothing about rendering
pub trait BaseGameExt {
    fn start_ports_and_positions(&self) -> Vec<(BasePort, Pt2)>;
}

for_each_game! {
    p::x, t => 

    impl BaseGameExt for BaseGame {
        fn start_ports_and_positions(&self) -> Vec<(BasePort, Pt2)> {
            match self {
                $($($p)*::$x(g) => g.start_ports_and_positions().into_iter()
                    .map(|(port, pos)| (port.wrap_base(), pos))
                    .collect()),*
            }
        }
    }
}

/// Renders a port collider, used for detecting whether the mouse is hovering over a port
pub fn render_port_collider() -> SvgElement {
    let svg_str = xml! {
        <g xmlns={SVG_NS} fill="transparent">
            <circle r="0.167"/>
        </g>
    }.to_string();
    parse_svg(&svg_str)
}

fn hsv_to_rgb(mut h: f32, s: f32, v: f32) -> Vec3f {
    h *= 6.0;
    let vec = Vec3f::from([
        ((h - 3.0).abs() - 1.0).clamp(0.0, 1.0),
        (-(h - 2.0).abs() + 2.0).clamp(0.0, 1.0),
        (-(h - 4.0).abs() + 2.0).clamp(0.0, 1.0),
    ]);
    (Vec3f::from([1.0, 1.0, 1.0]) * (1.0 - s) + vec * s) * v
}

pub const TOKEN_RADIUS: f64 = 0.1;

/// Renders a player token, given the player index and the number of players.
pub fn render_token(index: u32, num_players: u32, id_counter: &mut u64) -> String {
    let color = hsv_to_rgb(index as f32 / num_players as f32, 1.0, 1.0);
    let darker = color * 3.0 / 4.0;
    let color: Vec3u = na::try_convert(color * 255.0).expect("Color conversion failed");
    let darker: Vec3u = na::try_convert(darker * 255.0).expect("Color conversion failed");
    let id = {*id_counter += 1; *id_counter - 1};
    let result = xml!(
        <g xmlns={SVG_NS} transform="translate(0, 0)">
            <defs>
                <radialGradient id=("g"{id})>
                    <stop offset="0%" stop-color=("#"{color.x;02x}{color.y;02x}{color.z;02x})/>
                    <stop offset="100%" stop-color=("#"{darker.x;02x}{darker.y;02x}{darker.z;02x})/>
                </radialGradient>
            </defs>
            <circle r={TOKEN_RADIUS} fill=("url('#g"{id}"')")/>
        </g>
    ).to_string();
    result
}

/// Wraps the SVG in an `<svg>` element of a specific class.
/// TODO: The viewport is set so the svg fits snugly inside.
pub fn wrap_svg(svg: &str, class: &str) -> String {
    xml!(
        <svg xmlns={SVG_NS} class={class} viewBox={spaced!(-0.5, -0.5, 1, 1)}>{svg}</svg>
    ).to_string()
}