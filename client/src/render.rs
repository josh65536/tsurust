use std::f64::consts::TAU;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::{cell::Cell, marker::PhantomData};
use std::fmt::Debug;
use std::hash::Hash;
use common::{for_each_tile, nalgebra, nalgebra as na};

use common::math::{Mtx2, Pt2, Vec2f, Vec3f, Vec3u, pt2};
use common::nalgebra::{ComplexField, vector};
use common::{board::{BaseBoard, BasePort, Board, RectangleBoard}, for_each_board, for_each_game, game::{BaseGame, Game, PathGame}, math::Vec2, tile::{RegularTile, Tile}};
use common::board::{BaseTLoc, Port, TLoc};
use common::tile::{BaseGAct, BaseKind, BaseTile, Kind};
use getset::{CopyGetters, Getters, MutGetters};
use itertools::{Itertools, chain, iproduct, izip};
use specs::prelude::*;
use wasm_bindgen::{JsCast, prelude::Closure};
use web_sys::{DomParser, Element, MouseEvent, SupportedType, SvgElement, SvgGraphicsElement, SvgMatrix, SvgsvgElement};

use crate::game::GameWorld;
use crate::{SVG_NS, add_event_listener, console_log, document};

//fn create_svg_element<S: JsCast>(name: &str) -> S {
//    web_sys::window().unwrap().document().unwrap().create_element_ns(Some("http://www.w3.org/2000/svg"), name)
//        .expect("SVG element could not be created")
//        .dyn_into()
//        .expect("Wrong type specified")
//}

fn parse_svg(svg_str: &str) -> SvgElement {
    let svg = DomParser::new().unwrap().parse_from_string(&svg_str, SupportedType::ImageSvgXml)
        .expect("SVG could not be created");
    svg.document_element().expect("SVG doesn't have an element")
        .dyn_into().expect("SVG is not an SVG")
}

trait SvgMatrixExt {
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

/// Transformation component. Sets transform of other objects
#[derive(Clone, Debug)]
pub struct Transform {
    pub position: Pt2,
}

impl Component for Transform {
    type Storage = FlaggedStorage<Self, VecStorage<Self>>;
}

impl Transform {
    pub fn new(position: Pt2) -> Self {
        Self { position }
    }
}

/// Sets transforms
pub struct TransformSystem {
    reader_id: ReaderId<ComponentEvent>,
    changed: BitSet,
}

impl TransformSystem {
    pub fn new(world: &World) -> Self {
        let mut storage = world.write_storage::<Transform>();
        Self {
            reader_id: storage.register_reader(),
            changed: BitSet::new(),
        }
    }
}

impl<'a> System<'a> for TransformSystem {
    type SystemData = (ReadStorage<'a, Transform>, ReadStorage<'a, Model>);

    fn run(&mut self, (transforms, models): Self::SystemData) {
        self.changed.clear();

        for event in transforms.channel().read(&mut self.reader_id) {
            if let ComponentEvent::Modified(id) | ComponentEvent::Inserted(id) = event {
                self.changed.add(*id);
            }
        }

        for (transform, model, _) in (&transforms, &models, &self.changed).join() {
            let svg = document().get_element_by_id(&model.id).unwrap();
            svg.set_attribute("transform", &format!("translate({}, {})", transform.position.x, transform.position.y))
                .expect("Cannot change transform");
        }
    }
}

/// Labels an entity with a port
#[derive(Clone, Debug)]
pub struct PortLabel(pub BasePort);

impl Component for PortLabel {
    type Storage = DenseVecStorage<Self>;
}

/// Labels an entity with a tile location
#[derive(Clone, Debug)]
pub struct TLocLabel(pub BaseTLoc);

impl Component for TLocLabel {
    type Storage = DenseVecStorage<Self>;
}

/// Labels an entity with a tile
/// 
/// Group actions are *not* preapplied to the tile.
#[derive(Clone, Debug)]
pub struct TileLabel(pub BaseTile);

impl Component for TileLabel {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Clone, Debug, Getters, MutGetters, CopyGetters)]
pub struct TileSelect {
    /// Whether this entity is a selected tile
    selected: bool,
    #[getset(get = "pub")]
    kind: BaseKind,
    #[getset(get_copy = "pub", get_mut = "pub")]
    index: u32,
    action: BaseGAct,
}

impl TileSelect {
    fn new(kind: BaseKind, index: u32, action: BaseGAct) -> Self {
        Self { selected: false, kind, index, action }
    }
}

impl Component for TileSelect {
    type Storage = DenseVecStorage<Self>;
}

/// Rendering component
#[derive(Debug)]
pub struct Model {
    /// Id of the corresponding element
    id: String,
    order: i32,
    order_changed: bool,
}

impl Component for Model {
    type Storage = DenseVecStorage<Self>;
}

impl Model {
    pub const ORDER_BOARD: i32 = 0;
    pub const ORDER_TILE: i32 = 1;
    pub const ORDER_PLAYER_TOKEN: i32 = 2;
    pub const ORDER_TILE_HOVER: i32 = 3;

    /// Adds an element to a parent node, taking a counter that is used for the id and increments.
    /// Also takes a rendering order.
    /// Then returns a `Model`.
    pub fn new(elem: &Element, order: i32, parent: &Element, id: &mut u64) -> Self {
        elem.set_id(&id.to_string());
        *id += 1;
        parent.append_child(&elem).expect("Failed to add element");
        Model { id: elem.id(), order, order_changed: true }
    }
}

impl Drop for Model {
    /// Delete the SVG component
    fn drop(&mut self) {
        if let Some(element) = document().get_element_by_id(&self.id) {
            element.remove();
        }
    }
}

/// Mouse input tracker for the SVG region where the board shows
#[derive(Debug)]
pub struct BoardInput {
    /// Position of the mouse, in board space
    position: Pt2,
    position_raw: Rc<Cell<Pt2>>,
    callback: Closure<dyn FnMut(MouseEvent)>,
}

impl BoardInput {
    /// Constructs a `BoardInput` that gets mouse events from a specific SVG graphics element
    pub fn new(elem: &SvgGraphicsElement) -> Self {
        let position_raw = Rc::new(Cell::new(Pt2::origin()));
        let position_clone = Rc::clone(&position_raw);
        
        let elem_clone = elem.clone();
        let mousemove_listener = Closure::wrap(Box::new(move |e: MouseEvent| {
            let position = elem_clone.get_screen_ctm()
                .expect("Missing SVG matrix")
                .inverse().expect("Cannot inverse SVG matrix")
                .transform(pt2(e.x() as f64, e.y() as f64));
            position_clone.set(position);
        }) as Box<dyn FnMut(MouseEvent)>);
        elem.add_event_listener_with_callback("mousemove", mousemove_listener.as_ref().unchecked_ref())
            .expect("Failed to add input callback");

        Self {
            position: Pt2::origin(),
            position_raw,
            callback: mousemove_listener,
        }
    }

    fn position(&self) -> Pt2 {
        self.position
    }
}

/// Group action performed by a button press
#[derive(Clone, Copy, Debug)]
pub enum ButtonAction {
    Rotation{ num_times: i32 }
}

impl ButtonAction {
    /// Generate the corresponding group action
    pub fn group_action(&self, tile: &BaseTile) -> BaseGAct {
        match self {
            Self::Rotation{ num_times } => tile.rotation_action(*num_times)
        }
    }
}

impl Component for ButtonAction {
    type Storage = HashMapStorage<Self>;
}

/// An SVG is used for collision
#[derive(Debug)]
pub struct Collider {
    hovered: bool,
    clicked: bool,
    hovered_raw: Rc<Cell<bool>>,
    clicked_raw: Rc<Cell<bool>>,
    mouseover_listener: Closure<dyn FnMut(MouseEvent)>,
    mouseout_listener: Closure<dyn FnMut(MouseEvent)>,
    click_listener: Closure<dyn FnMut(MouseEvent)>,
}

impl Component for Collider {
    type Storage = DenseVecStorage<Self>;
}

impl Collider {
    pub const ORDER_START_PORT: i32 = -(i32::MIN / 2) + 1;
    pub const ORDER_TILE_LOC: i32 = -(i32::MIN / 2) + 0;

    /// Constructs a collider.
    /// Takes an element to insert callbacks into
    pub fn new(elem: &Element) -> Self {
        let hovered_raw = Rc::new(Cell::new(false));
        let hovered_clone = Rc::clone(&hovered_raw);
        let mouseover_listener = Closure::wrap(Box::new(move |e: MouseEvent| {
            hovered_clone.set(true);
        }) as Box<dyn FnMut(MouseEvent)>);
        let hovered_clone = Rc::clone(&hovered_raw);
        let mouseout_listener = Closure::wrap(Box::new(move |e: MouseEvent| {
            hovered_clone.set(false);
        }) as Box<dyn FnMut(MouseEvent)>);

        elem.add_event_listener_with_callback("mouseover", mouseover_listener.as_ref().unchecked_ref())
            .expect("Failed to add collider callback");
        elem.add_event_listener_with_callback("mouseout", mouseout_listener.as_ref().unchecked_ref())
            .expect("Failed to add collider callback");

        let clicked_raw = Rc::new(Cell::new(false));
        let clicked_clone = Rc::clone(&clicked_raw);
        let click_listener = Closure::wrap(Box::new(move |e: MouseEvent| {
            clicked_clone.set(true);
        }) as Box<dyn FnMut(MouseEvent)>);

        elem.add_event_listener_with_callback("click", click_listener.as_ref().unchecked_ref())
            .expect("Failed to add collider callback");

        Collider {
            hovered: false,
            clicked: false,
            hovered_raw,
            clicked_raw,
            mouseover_listener,
            mouseout_listener,
            click_listener,
        }
    }

    /// Whether the collider is being hovered over
    pub fn hovered(&self) -> bool {
        self.hovered
    }

    /// Whether the collider is being clicked on this frame
    pub fn clicked(&self) -> bool {
        self.clicked
    }
}

/// Updates collider inputs
pub struct ColliderInputSystem;

impl<'a> System<'a> for ColliderInputSystem {
    // Option<Write<..>> is used even though the resource is strictly required
    // because BoardInput doesn't have a default
    type SystemData = (WriteStorage<'a, Collider>, Option<Write<'a, BoardInput>>);

    fn run(&mut self, (mut colliders, input): Self::SystemData) {
        for collider in (&mut colliders).join() {
            collider.hovered = collider.hovered_raw.get();
            collider.clicked = collider.clicked_raw.get();
            collider.clicked_raw.set(false);
        }

        let mut input = input.expect("Missing BoardInput");
        input.position = input.position_raw.get();
    }
}

/// Orders nodes to render
pub struct SvgOrderSystem;

impl<'a> System<'a> for SvgOrderSystem {
    type SystemData = WriteStorage<'a, Model>;

    fn run(&mut self, mut models: Self::SystemData) {
        // Reorder nodes, since z-index isn't consistently supported
        let groups = (&mut models).join()
            .map(|m| (&m.id, m.order, &mut m.order_changed))
            .sorted_by_key(|(svg_id, _, _)| {
                document().get_element_by_id(svg_id).unwrap()
                    .parent_element().expect("SVG node parents should have ids for sorting purposes").id()
            })
            .group_by(|(svg_id, _, _)| {
                document().get_element_by_id(svg_id).unwrap()
                    .parent_element().expect("SVG node parents should have ids for sorting purposes").id()
            });

        for (parent_id, group) in groups.into_iter() {
            let mut values = group.collect_vec();
            // Sort only if some node changed order
            if values.iter().all(|(_, _, order_changed)| !**order_changed) {
                continue;
            }

            values.sort_by_key(|(_, order, _)| *order);
            let parent = document().get_element_by_id(&parent_id).expect("SVG node unexpectedly removed");
            for (svg_id, order, order_changed) in values {
                let elem = document().get_element_by_id(svg_id).expect("SVG node unexpectedly removed");
                let node = parent.remove_child(&elem).expect("Failed to reorder");
                parent.append_child(&node).expect("Failed to reorder");
                *order_changed = false;
            }
        }
    }
}

/// A place where the player token can get added
#[derive(Clone, Copy, Debug, Default)]
pub struct TokenSlot;

impl Component for TokenSlot {
    type Storage = NullStorage<Self>;
}

/// The token that's being placed
#[derive(Clone, Copy, Debug, Default)]
pub struct TokenToPlace;

impl Component for TokenToPlace {
    type Storage = NullStorage<Self>;
}

/// The port a token was placed on
#[derive(Clone, Debug, Default)]
pub struct PlacedPort(pub Option<BasePort>);

#[derive(Clone, Copy, Debug, Default)]
pub struct RunPlaceTokenSystem(pub bool);

pub struct PlaceTokenSystem;

#[derive(SystemData)]
pub struct PlaceTokenSystemData<'a> {
    run: Read<'a, RunPlaceTokenSystem>,
    placed_port: Write<'a, PlacedPort>,
    tokens: ReadStorage<'a, TokenToPlace>,
    token_slots: ReadStorage<'a, TokenSlot>,
    colliders: ReadStorage<'a, Collider>,
    ports: ReadStorage<'a, PortLabel>,
    transforms: WriteStorage<'a, Transform>,
    input: Option<Read<'a, BoardInput>>,
}

impl<'a> System<'a> for PlaceTokenSystem {
    type SystemData = PlaceTokenSystemData<'a>;
    
    fn run(&mut self, mut data: Self::SystemData) {
        if !data.run.0 { return }

        let position = (&data.token_slots, &data.colliders, &data.transforms).join()
            .flat_map(|(_, collider, transform)| {
                collider.hovered().then(|| transform.position)
            })
            .next();

        for (_, transform) in (&data.tokens, &mut data.transforms).join() {
            transform.position = if let Some(position) = position {
                position
            } else {
                data.input.as_ref().expect("Missing BoardInput").position()
            }
        }

        for (_, collider, port) in (&data.token_slots, &data.colliders, &data.ports).join() {
            if collider.clicked() {
                data.placed_port.0 = Some(port.0.clone());
                break;
            }
        }
    }
}

/// A place where the player token can get added
#[derive(Clone, Copy, Debug, Default)]
pub struct TileSlot;

impl Component for TileSlot {
    type Storage = NullStorage<Self>;
}

/// The token that's being placed
#[derive(Clone, Copy, Debug, Default)]
pub struct TileToPlace;

impl Component for TileToPlace {
    type Storage = NullStorage<Self>;
}

/// The location a tile was placed on
#[derive(Clone, Debug, Default)]
pub struct PlacedTLoc(pub Option<BaseTLoc>);

#[derive(Clone, Copy, Debug, Default)]
pub struct RunPlaceTileSystem(pub bool);

pub struct PlaceTileSystem;

#[derive(SystemData)]
pub struct PlaceTileSystemData<'a> {
    run: Read<'a, RunPlaceTileSystem>,
    placed_loc: Write<'a, PlacedTLoc>,
    tiles: ReadStorage<'a, TileToPlace>,
    tile_slots: ReadStorage<'a, TileSlot>,
    colliders: ReadStorage<'a, Collider>,
    locs: ReadStorage<'a, TLocLabel>,
    transforms: WriteStorage<'a, Transform>,
    input: Option<Read<'a, BoardInput>>,
}

impl<'a> System<'a> for PlaceTileSystem {
    type SystemData = PlaceTileSystemData<'a>;
    
    fn run(&mut self, mut data: Self::SystemData) {
        if !data.run.0 { return }

        let position = (&data.tile_slots, &data.colliders, &data.transforms).join()
            .flat_map(|(_, collider, transform)| {
                collider.hovered().then(|| transform.position)
            })
            .next();

        for (_, transform) in (&data.tiles, &mut data.transforms).join() {
            transform.position = if let Some(position) = position {
                position
            } else {
                data.input.as_ref().expect("Missing BoardInput").position()
            }
        }

        for (_, collider, loc) in (&data.tile_slots, &data.colliders, &data.locs).join() {
            if collider.clicked() {
                data.placed_loc.0 = Some(loc.0.clone());
                break;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RunSelectTileSystem(pub bool);

pub struct SelectTileSystem;

/// The tile that's currently selected, paired with its index and group action
/// into the list of tiles the player has of the same kind.
/// 
/// The group is *not* preapplied to the tile.
#[derive(Clone, Debug, Default)]
pub struct SelectedTile(pub u32, pub Option<BaseGAct>, pub Option<BaseTile>);

#[derive(SystemData)]
pub struct SelectTileSystemData<'a> {
    run: Read<'a, RunSelectTileSystem>,
    selected_tile: Write<'a, SelectedTile>,
    models: ReadStorage<'a, Model>,
    colliders: ReadStorage<'a, Collider>,
    tiles: ReadStorage<'a, TileLabel>,
    tile_selects: WriteStorage<'a, TileSelect>,
    button_actions: ReadStorage<'a, ButtonAction>,
}

impl<'a> System<'a> for SelectTileSystem {
    type SystemData = SelectTileSystemData<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        if !data.run.0 { return; }

        // Edit group action if necessary
        let selected_tile = &mut *data.selected_tile;
        if let (Some(action), Some(tile)) = (&mut selected_tile.1, &selected_tile.2) {
            for (collider, button_action) in (&data.colliders, &data.button_actions).join() {
                if collider.clicked() {
                    *action = action.compose(&button_action.group_action(tile));
                }
            }
        }

        // Only do something when the selection is modified
        if (&data.colliders, &data.tile_selects).join().all(|(c, _)| !c.clicked()) {
            return;
        }

        let mut found_selected = false;

        for (collider, tile, tile_select) in (&data.colliders, &data.tiles, &mut data.tile_selects).join() {
            if found_selected {
                tile_select.selected = false;
                continue;
            }

            tile_select.selected = collider.clicked();
            if collider.clicked() {
                found_selected = true;
                data.selected_tile.0 = tile_select.index;
                data.selected_tile.1 = Some(tile_select.action.clone());
                data.selected_tile.2 = Some(tile.0.clone());
            }
        }

        // Update selection visualization
        for (model, tile_select) in (&data.models, &data.tile_selects).join() {
            let elem = document().get_element_by_id(&model.id).expect("Missing model element");
            elem.set_attribute(
                "class", 
                if tile_select.selected { "tile-selected" } else { "tile-unselected" }
            ).expect("Cannot set tile select style");
        }
    }
}

/// Extension trait for Board, mainly for rendering since
/// the server should know nothing about rendering
pub trait BoardExt: Board {
    fn render(&self) -> SvgElement;

    fn port_position(&self, port: &Self::Port) -> Pt2;

    fn loc_position(&self, loc: &Self::TLoc) -> Pt2;

    /// Render the collider for a specific tile location.
    fn render_collider(&self, loc: &Self::TLoc) -> SvgElement;

    /// Creates an entity (mainly for collision detection) at a specific tile location.
    fn create_loc_collider_entity(&self, loc: &Self::TLoc, world: &mut World, id_counter: &mut u64) -> Entity;
}

impl BoardExt for RectangleBoard {
    fn render(&self) -> SvgElement {
        let svg_str = format!(r##"<g xmlns="{}" class="rectangular-board">"##, SVG_NS) +
            &chain!(
                iproduct!(0..self.height(), 0..self.width()).map(|(y, x)|
                    format!(r##"<rect x="{}" y="{}" width="1" height="1"/>"##, x, y)),
                self.boundary_ports().into_iter().map(|(min, d)| {
                    let v = self.port_position(&(min, d));
                    let dx = if d.x == 0 { 0.1 } else { 0.0 };
                    let dy = if d.y == 0 { 0.1 } else { 0.0 };
                    format!(r##"<line x1="{}" x2="{}" y1="{}" y2="{}" class="rectangular-board-notch"/>"##, v.x - dx, v.x + dx, v.y - dy, v.y + dy)
                })
            )
                .join("") +
            r##"</g>"##;

        parse_svg(&svg_str)
    }

    fn port_position(&self, port: &<Self as Board>::Port) -> Pt2 {
        port.0.cast::<f64>() + port.1.cast::<f64>() / (self.ports_per_edge() + 1) as f64
    }

    fn loc_position(&self, loc: &Self::TLoc) -> Pt2 {
        loc.cast() + vector![0.5, 0.5]
    }

    fn render_collider(&self, loc: &Self::TLoc) -> SvgElement {
        let svg_str = format!(concat!(
            r##"<g xmlns="{}" fill="transparent">"##,
            r##"<rect x="-0.5" y="-0.5" width="1" height="1"/>"##,
            r##"</g>"##
        ), SVG_NS);
        parse_svg(&svg_str)
    }

    fn create_loc_collider_entity(&self, loc: &Self::TLoc, world: &mut World, id_counter: &mut u64) -> Entity {
        let svg = self.render_collider(loc);
        world.create_entity()
            .with(Model::new(&svg, Collider::ORDER_TILE_LOC, &GameWorld::svg_root(), id_counter))
            .with(Collider::new(&svg))
            .with(Transform::new(self.loc_position(loc)))
            .with(TLocLabel(loc.clone().wrap_base()))
            .with(TileSlot)
            .build()
    }
}

/// Extension trait for BaseBoard, mainly for rendering since
/// the server should know nothing about rendering
pub trait BaseBoardExt {
    fn render(&self) -> SvgElement;
    
    fn port_position(&self, port: &BasePort) -> Pt2;

    fn loc_position(&self, loc: &BaseTLoc) -> Pt2;

    /// Creates an entity (mainly for collision detection) at a specific tile location.
    fn create_loc_collider_entity(&self, loc: &BaseTLoc, world: &mut World, id_counter: &mut u64) -> Entity;
}

for_each_board! {
    p::x, t => 

    impl BaseBoardExt for BaseBoard {
        fn render(&self) -> SvgElement {
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
    format!(r##"<polygon points="{}"/>"##, poly_str)
}

/// Extension trait for Tile, mainly for rendering since
/// the server should know nothing about rendering
pub trait TileExt: Tile {
    fn render(&self) -> SvgElement;
}

impl<const EDGES: u32> TileExt for RegularTile<EDGES> {
    fn render(&self) -> SvgElement {
        if self.visible() {
            let connections = (0..self.num_ports()).map(|i| self.output(i)).collect_vec();
            let mut covered = vec![false; connections.len()];
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
                    format!(concat!(
                        r##"<path class="regular-tile-path-outer" d="M {0},{1} C {2},{3} {4},{5} {6},{7}"/>"##,
                        r##"<path class="regular-tile-path-inner" d="M {0},{1} C {2},{3} {4},{5} {6},{7}"/>"##,
                    ), p0.x, p0.y, p1.x, p1.y, p2.x, p2.y, p3.x, p3.y)
                })
                .join("");

            let poly_str = regular_polygon_svg_str(EDGES);
            let svg_str = format!(concat!(
                r##"<g xmlns="{}" class="regular-tile-visible">"##,
                "{}{}",
                r##"</g>"##,
            ), SVG_NS, poly_str, path_str);
            parse_svg(&svg_str)
        } else {
            let poly_str = regular_polygon_svg_str(EDGES);
            let svg_str = format!(concat!(
                r##"<g xmlns="{}" class="regular-tile-hidden">"##,
                r##"{}"##,
                r##"</g>"##,
            ), SVG_NS, poly_str);
            parse_svg(&svg_str)
        }
    }
}

/// Extension trait for BaseTile, mainly for rendering since
/// the server should know nothing about rendering
pub trait BaseTileExt {
    fn render(&self) -> SvgElement;

    fn create_hand_entity(&self, index: u32, action: &BaseGAct, world: &mut World, id_counter: &mut u64) -> Entity;

    fn create_board_entity_common<'a>(&self, world: &'a mut World, id_counter: &mut u64) -> EntityBuilder<'a>;

    fn create_to_place_entity(&self, action: &BaseGAct, world: &mut World, id_counter: &mut u64) -> Entity;

    fn create_on_board_entity(&self, board: &BaseBoard, loc: &BaseTLoc, world: &mut World, id_counter: &mut u64) -> Entity;
}

for_each_tile! {
    p::x, t => 

    impl BaseTileExt for BaseTile {
        fn render(&self) -> SvgElement {
            match self { $($($p)*::$x(b) => b.render()),* }
        }

        fn create_hand_entity(&self, index: u32, action: &BaseGAct, world: &mut World, id_counter: &mut u64) -> Entity {
            match self { $($($p)*::$x(b) => {
                let svg = self.apply_action(action).render();
                let wrapper = wrap_svg(&svg.dyn_into().unwrap(), 128);
                wrapper.set_attribute("class", "tile-unselected").expect("Cannot set tile select class");
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

        fn create_to_place_entity(&self, action: &BaseGAct, world: &mut World, id_counter: &mut u64) -> Entity {
            match self { $($($p)*::$x(b) => {
                let svg = self.apply_action(action).render();
                self.create_board_entity_common(world, id_counter)
                    .with(Model::new(&svg, Model::ORDER_TILE_HOVER, &GameWorld::svg_root(), id_counter))
                    .with(TileToPlace)
                    .with(Transform::new(Pt2::origin()))
                    .build()
            }),* }
        }

        fn create_on_board_entity(&self, board: &BaseBoard, loc: &BaseTLoc, world: &mut World, id_counter: &mut u64) -> Entity {
            match self { $($($p)*::$x(b) => {
                let svg = self.render();
                self.create_board_entity_common(world, id_counter)
                    .with(Model::new(&svg, Model::ORDER_TILE, &GameWorld::svg_root(), id_counter))
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
    let svg_str = format!(concat!(
        r##"<g xmlns="{0}" fill="transparent">"##,
        r##"<circle r="0.167"/>"##,
        r##"</g>"##,
    ), SVG_NS);
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

/// Renders a player token, given the player index and the number of players.
pub fn render_token(index: u32, num_players: u32, id_counter: &mut u64) -> SvgElement {
    let color = hsv_to_rgb(index as f32 / num_players as f32, 1.0, 1.0);
    let darker = color * 3.0 / 4.0;
    let color: Vec3u = na::try_convert(color * 255.0).expect("Color conversion failed");
    let darker: Vec3u = na::try_convert(darker * 255.0).expect("Color conversion failed");
    let svg_str = format!(concat!(
        r##"<g xmlns="{0}" transform="translate(0, 0)">"##,
        r##"<defs>"##,
        r##"<radialGradient id="g{7}">"##,
        r##"<stop offset="0%" stop-color="#{1:02x}{2:02x}{3:02x}"/>"##,
        r##"<stop offset="100%" stop-color="#{4:02x}{5:02x}{6:02x}"/>"##,
        r##"</radialGradient>"##,
        r##"</defs>"##,
        r##"<circle r="0.1" fill="url('#g{7}')"/>"##,
        r##"</g>"##
    ), SVG_NS, color.x, color.y, color.z, darker.x, darker.y, darker.z, {*id_counter += 1; *id_counter - 1});
    parse_svg(&svg_str)
}

/// Wraps the SVG in an `<svg>` element of a specific size.
/// The viewport is set so the svg fits snugly inside.
pub fn wrap_svg(svg: &SvgGraphicsElement, size: u32) -> SvgElement {
    let bbox = svg.get_b_box().expect("Cannot get bounding box");
    let wrapper_str = format!(concat!(
        r##"<svg xmlns="{0}" width="{1}" height="{1}" viewBox="{2} {3} {4} {5}">"##,
        r##"</svg>"##
    ), SVG_NS, size, -0.5, -0.5, 1, 1);//bbox.x(), bbox.y(), bbox.width(), bbox.height());
    let wrapper = parse_svg(&wrapper_str);
    wrapper.append_child(svg).expect("Cannot wrap svg");
    wrapper
}