//! The ECS systems page: one card per system in the battle's `SimTick`
//! schedule. Each card runs its own small `World` with the real schedule
//! (the same code the battle runs), set up so that card's system is the one
//! doing the visible work, and draws an overlay of what that system reads
//! or writes.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::message::Messages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::flow::Phase;
use super::theme::*;
use super::ui::{BtnKind, BtnLabel, Enabled, Selected, button_for, card_title, panel, row, text};
use crate::sim::battle::{Battle, DT, EndReason, SimEvent, SimSettings, Tuning};
use crate::sim::components::*;
use crate::sim::formation::{self, Formation, SHIP_DART};
use crate::sim::spatial::SpatialHash;
use crate::voronoi::raster::{Canvas, Rgb};

/// Each card's picture, in pixels.
const IMG_W: u32 = 560;
const IMG_H: u32 = 320;
/// Most sim ticks a card runs in one frame (so a slow frame can't snowball).
const MAX_TICKS: u32 = 8;

const C_BG: Rgb = [0x0b, 0x0f, 0x18];
const C_GRID: Rgb = [0x2a, 0x33, 0x50];
const C_TEXT: Rgb = [0xd7, 0xde, 0xef];
const C_DIM: Rgb = [0x88, 0x92, 0xab];
const C_GOLD: Rgb = [0xff, 0xd1, 0x66];
const C_GREEN: Rgb = [0x4c, 0xd9, 0x7b];
const C_SIDE: [Rgb; 2] = [[0x4a, 0x9e, 0xff], [0xff, 0x6a, 0x4a]];
const C_ROUTED: Rgb = [0x5d, 0x64, 0x78];

/// The systems, in schedule order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Demo {
    TickClock,
    RebuildHash,
    Commander,
    SteerDivisions,
    SteerShips,
    Integrate,
    Fire,
    RemoveFled,
    CloseRanks,
    Morale,
    CheckEnd,
}

impl Demo {
    pub const ALL: [Demo; 11] = [
        Demo::TickClock,
        Demo::RebuildHash,
        Demo::Commander,
        Demo::SteerDivisions,
        Demo::SteerShips,
        Demo::Integrate,
        Demo::Fire,
        Demo::RemoveFled,
        Demo::CloseRanks,
        Demo::Morale,
        Demo::CheckEnd,
    ];

    fn name(self) -> &'static str {
        match self {
            Demo::TickClock => "tick_clock",
            Demo::RebuildHash => "rebuild_hash",
            Demo::Commander => "ai::commander",
            Demo::SteerDivisions => "steer_divisions",
            Demo::SteerShips => "steer_ships",
            Demo::Integrate => "integrate",
            Demo::Fire => "fire",
            Demo::RemoveFled => "remove_fled",
            Demo::CloseRanks => "close_ranks",
            Demo::Morale => "morale",
            Demo::CheckEnd => "check_end",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Demo::TickClock => {
                "Writes Battle.time. Every tick adds DT = 0.05 s, so 20 ticks make a second of battle time whatever the \
                 frame rate. The bar shows the ticks of the current second; timers elsewhere (morale once a second, the AI \
                 every 2 s, close ranks every 3 s) count from this clock."
            }
            Demo::RebuildHash => {
                "Reads every ship's Pos, Heading, Side and InDivision and writes the SpatialHash resource: ships bucketed \
                 into 110-unit cells (shaded by how many they hold) plus each division's centroid (+). The ring is a \
                 for_each_within query: only the cells it overlaps are searched, and the ships it finds are ringed."
            }
            Demo::Commander => {
                "Every 2 s, for each AI side, reads the divisions and writes their Order. Engage = arrow to the target \
                 division; Move = dashed line to the point. Here both sides are commanded: pinning, then flanking the \
                 enemy that is already pinned."
            }
            Demo::SteerDivisions => {
                "Moves each Division's anchor and wheels its facing toward its Order. The Blue division gets a new Move \
                 order every 9 s (gold ring). The anchor (+) moves at anchor speed and the facing (arrow) turns at a \
                 limited rate; the ships only follow it."
            }
            Demo::SteerShips => {
                "Sets each ship's Vel and Heading: steer for its Slot in the division's frame (the lines go from ship to \
                 slot) and turn toward its target within the aim limit. Every 6 s this division re-forms into the next \
                 formation and turns, so every ship gets a new slot."
            }
            Demo::Integrate => {
                "Moves ships by Vel, keeps friends apart and never lets a ship move into an enemy's space (the faint rings: \
                 the blocking distance). Guns are off here (hit chance 0), so the Blue column charging the Red line simply \
                 stops against it: an intact line blocks."
            }
            Demo::Fire => {
                "Every ship whose Weapon has reloaded shoots the nearest enemy in its gun cone (one Blue ship's cone is \
                 drawn). Hit chance depends on the arc it lands in: front 12%, flank 25%, rear 40%. Hits (bright) cost hull; \
                 misses are faint. Red has a second division on Blue's flank."
            }
            Demo::RemoveFled => {
                "Despawns ships of a Routed division once they leave Battle.bounds (the box) and counts them as fled. The \
                 Blue division starts routed and runs for its home edge (left). The other Blue division stays, so the \
                 battle goes on."
            }
            Demo::CloseRanks => {
                "Every 3 s, deep formations (wedge, column) re-form: the rear ships step into the gaps at the front. The Blue \
                 wedge (left) closes up as it loses ships; the Blue line (right) is too thin to do that and keeps its holes. \
                 Rings are the empty slots."
            }
            Demo::Morale => {
                "Once a second, from each Division's shock, losses, the fleet's losses and its flagship, works out stress and \
                 moves its Morale: Steady → Shaken above 0.55 (gold, fires slower), Shaken → Routed above 0.9 (grey, runs). \
                 Blue is shot from the front and the flank; the bars are each division's shock."
            }
            Demo::CheckEnd => {
                "Counts the ships still fighting (not routed) per side, writes Battle.lost and ends the battle once a side \
                 has none left, nobody has fired for 45 s, or time runs out. The bars are each side's fighting ships; the \
                 frame turns the winner's colour. Hulls are 2 here (not 5) so it ends sooner; it restarts 3 s later."
            }
        }
    }
}

/// One card: its world and picture.
struct Card {
    demo: Demo,
    world: World,
    image: Handle<Image>,
    /// Shots and explosions to draw: (from, to, side, hit, age).
    shots: Vec<(Vec2, Vec2, usize, bool, f32)>,
    booms: Vec<(Vec2, f32)>,
    /// Battle time when the card last changed something by script.
    script_t: f32,
    script_step: usize,
    /// Seconds since the battle ended (restart after a pause).
    over_for: f32,
    restarts: u32,
}

#[derive(Resource)]
pub struct EcsLabs {
    cards: Vec<Card>,
    paused: bool,
    speed: u32,
    acc: f32,
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub enum EcsAction {
    Pause,
    Speed(u32),
    Restart(usize),
    RestartAll,
}

#[derive(Component)]
pub struct EcsPage;
#[derive(Component)]
pub struct EcsStatus(usize);

fn blank_image() -> Image {
    let mut img = Image::new(
        Extent3d { width: IMG_W, height: IMG_H, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (IMG_W * IMG_H * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::linear();
    img
}

// --- Building the worlds ---

/// The sim's own empty world; the commander card lets the AI run both sides.
fn empty_world(seed: u32, bounds: Rect, tuning: Tuning, commander: bool) -> World {
    let mut world = crate::sim::open_world(tuning, seed, bounds);
    if commander {
        world.resource_mut::<SimSettings>().ai = [crate::sim::battle::AiKind::Commander; 2];
    }
    world
}

fn spawn_division(world: &mut World, side: usize, form: Formation, anchor: Vec2, facing: f32, ships: usize, order: Order) -> Entity {
    crate::sim::spawn_division(world, side, form, anchor, facing, ships, order)
}

const EAST: f32 = 0.0;
const WEST: f32 = std::f32::consts::PI;
const NORTH: f32 = std::f32::consts::FRAC_PI_2;
const SOUTH: f32 = -std::f32::consts::FRAC_PI_2;

/// A division that only keeps the battle from ending (off the picture).
fn spawn_bystander(world: &mut World) {
    spawn_division(world, 1, Formation::Line, Vec2::new(1800.0, 0.0), WEST, 4, Order::Hold);
}

fn build(demo: Demo, seed: u32) -> World {
    let wide = Rect::new(-2000.0, -1200.0, 2000.0, 1200.0);
    let mut t = Tuning::default();
    match demo {
        Demo::Integrate => t.hit = [0.0; 3],
        // Lighter hulls so the fight reaches its end in under a minute.
        Demo::CheckEnd => t.hull = 2.0,
        _ => {}
    }
    let mut w = empty_world(seed, wide, t, demo == Demo::Commander);
    match demo {
        Demo::TickClock => {
            spawn_division(&mut w, 0, Formation::Line, Vec2::new(-260.0, 0.0), EAST, 16, Order::Advance);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(260.0, 0.0), WEST, 16, Order::Advance);
        }
        Demo::RebuildHash => {
            spawn_division(&mut w, 0, Formation::Wedge, Vec2::new(-280.0, 40.0), EAST, 24, Order::Advance);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(200.0, 0.0), WEST, 24, Order::Hold);
            spawn_division(&mut w, 1, Formation::Column, Vec2::new(120.0, -170.0), WEST, 12, Order::Hold);
        }
        Demo::Commander => {
            for (k, y) in [-150.0, 0.0, 150.0].into_iter().enumerate() {
                let f = if k == 1 { Formation::Wedge } else { Formation::Line };
                spawn_division(&mut w, 0, f, Vec2::new(-380.0, y), EAST, 14, Order::Hold);
                spawn_division(&mut w, 1, Formation::Line, Vec2::new(380.0, y * 1.2), WEST, 14, Order::Hold);
            }
        }
        Demo::SteerDivisions => {
            spawn_division(&mut w, 0, Formation::Line, Vec2::new(-200.0, -80.0), EAST, 14, Order::Hold);
            spawn_bystander(&mut w);
        }
        Demo::SteerShips => {
            spawn_division(&mut w, 0, Formation::Line, Vec2::ZERO, EAST, 24, Order::Hold);
            spawn_bystander(&mut w);
        }
        Demo::Integrate => {
            spawn_division(&mut w, 0, Formation::Column, Vec2::new(-260.0, 0.0), EAST, 18, Order::Charge);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(90.0, 0.0), WEST, 24, Order::Hold);
        }
        Demo::Fire => {
            spawn_division(&mut w, 0, Formation::Line, Vec2::new(-70.0, 0.0), EAST, 20, Order::Hold);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(70.0, 0.0), WEST, 20, Order::Hold);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(-80.0, 190.0), SOUTH, 12, Order::Hold);
        }
        Demo::RemoveFled => {
            w.resource_mut::<Battle>().bounds = Rect::new(-330.0, -170.0, 330.0, 170.0);
            let fleeing = spawn_division(&mut w, 0, Formation::Line, Vec2::new(150.0, 30.0), WEST, 18, Order::Hold);
            w.get_mut::<Division>(fleeing).unwrap().morale = Morale::Routed;
            spawn_division(&mut w, 0, Formation::Line, Vec2::new(-200.0, -110.0), EAST, 8, Order::Hold);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(250.0, -110.0), WEST, 8, Order::Hold);
        }
        Demo::CloseRanks => {
            spawn_division(&mut w, 0, Formation::Wedge, Vec2::new(-80.0, 100.0), EAST, 25, Order::Hold);
            spawn_division(&mut w, 0, Formation::Line, Vec2::new(-60.0, -100.0), EAST, 20, Order::Hold);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(70.0, 100.0), WEST, 26, Order::Hold);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(80.0, -100.0), WEST, 26, Order::Hold);
        }
        Demo::Morale => {
            spawn_division(&mut w, 0, Formation::Line, Vec2::new(-80.0, -20.0), EAST, 20, Order::Hold);
            spawn_division(&mut w, 0, Formation::Line, Vec2::new(-150.0, -170.0), EAST, 12, Order::Hold);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(70.0, -20.0), WEST, 22, Order::Hold);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(-80.0, 125.0), SOUTH, 20, Order::Hold);
        }
        Demo::CheckEnd => {
            spawn_division(&mut w, 0, Formation::Wedge, Vec2::new(-200.0, 60.0), EAST, 16, Order::Charge);
            spawn_division(&mut w, 0, Formation::Line, Vec2::new(-140.0, -90.0), EAST, 12, Order::Advance);
            spawn_division(&mut w, 1, Formation::Line, Vec2::new(120.0, 0.0), WEST, 30, Order::Advance);
        }
    }
    use bevy::ecs::system::RunSystemOnce;
    let _ = w.run_system_once(crate::sim::systems::rebuild_hash);
    w
}

/// The world area each card shows: centre and width.
fn view_of(demo: Demo) -> (Vec2, f32) {
    match demo {
        Demo::Commander => (Vec2::ZERO, 1100.0),
        Demo::SteerDivisions => (Vec2::ZERO, 820.0),
        Demo::SteerShips => (Vec2::ZERO, 420.0),
        Demo::Fire | Demo::Morale => (Vec2::new(-20.0, 0.0), 640.0),
        Demo::CloseRanks => (Vec2::ZERO, 560.0),
        Demo::CheckEnd => (Vec2::ZERO, 620.0),
        Demo::RemoveFled => (Vec2::ZERO, 760.0),
        _ => (Vec2::ZERO, 800.0),
    }
}

impl Card {
    fn new(demo: Demo, image: Handle<Image>, restarts: u32) -> Self {
        let seed = 17 + restarts * 31 + demo as u32 * 101;
        Self { demo, world: build(demo, seed), image, shots: Vec::new(), booms: Vec::new(), script_t: 0.0, script_step: 0, over_for: 0.0, restarts }
    }

    fn restart(&mut self) {
        *self = Card::new(self.demo, self.image.clone(), self.restarts + 1);
    }

    /// Scripted orders some cards give on a timer.
    fn script(&mut self) {
        let time = self.world.resource::<Battle>().time;
        let every = match self.demo {
            Demo::SteerDivisions => 9.0,
            Demo::SteerShips => 6.0,
            _ => return,
        };
        if time < self.script_t {
            return;
        }
        self.script_t = time + every;
        let k = self.script_step;
        self.script_step += 1;
        let Some(div) = self.world.query_filtered::<Entity, With<Division>>().iter(&self.world).find(|e| {
            self.world.get::<Division>(*e).is_some_and(|d| d.side == 0)
        }) else {
            return;
        };
        match self.demo {
            Demo::SteerDivisions => {
                const WAY: [(Vec2, f32); 4] = [
                    (Vec2::new(180.0, -80.0), NORTH),
                    (Vec2::new(180.0, 140.0), WEST),
                    (Vec2::new(-220.0, 140.0), SOUTH),
                    (Vec2::new(-220.0, -100.0), EAST),
                ];
                let (to, facing) = WAY[k % WAY.len()];
                self.world.get_mut::<Division>(div).unwrap().order = Order::MoveTo { to, facing };
            }
            Demo::SteerShips if k > 0 => {
                let f = Formation::PLAYER[k % Formation::PLAYER.len()];
                let facing = [EAST, 0.6, -0.4, EAST, 1.2, -0.8][k % 6];
                self.world.get_mut::<Division>(div).unwrap().order = Order::MoveTo { to: Vec2::ZERO, facing };
                crate::sim::set_formation(&mut self.world, div, f);
            }
            _ => {}
        }
    }

    fn tick(&mut self, ticks: u32) {
        for (_, _, _, _, age) in &mut self.shots {
            *age += DT * ticks as f32;
        }
        for (_, age) in &mut self.booms {
            *age += DT * ticks as f32;
        }
        self.shots.retain(|s| s.4 < if s.3 { LASER_HIT_SECS } else { LASER_MISS_SECS });
        self.booms.retain(|b| b.1 < BOOM_SECS);
        for _ in 0..ticks {
            if self.world.resource::<Battle>().over() {
                self.over_for += DT;
                if self.over_for > 3.0 {
                    self.restart();
                    return;
                }
                continue;
            }
            self.script();
            crate::sim::step(&mut self.world);
            let events: Vec<SimEvent> = self.world.resource_mut::<Messages<SimEvent>>().drain().collect();
            for ev in events {
                match ev {
                    SimEvent::Shot { from, to, side, hit, .. } => self.shots.push((from, to, side, hit, 0.0)),
                    SimEvent::Destroyed { pos, .. } => self.booms.push((pos, 0.0)),
                    _ => {}
                }
            }
            // The scripted cards would otherwise end at the time limit.
            if matches!(self.demo, Demo::SteerDivisions | Demo::SteerShips | Demo::TickClock) && self.world.resource::<Battle>().time > 120.0 {
                self.restart();
                return;
            }
            // Cards whose point is made once everyone has left restart a bit later.
            if self.demo == Demo::RemoveFled && self.world.resource::<Battle>().fled[0] >= 18 {
                self.over_for += DT;
                if self.over_for > 2.0 {
                    self.restart();
                    return;
                }
            }
        }
    }
}

// --- Drawing ---

struct Cam {
    min: Vec2,
    scale: f32,
}

impl Cam {
    fn of(demo: Demo) -> Self {
        let (c, w) = view_of(demo);
        let scale = IMG_W as f32 / w;
        let h = IMG_H as f32 / scale;
        Self { min: Vec2::new(c.x - w / 2.0, c.y + h / 2.0), scale }
    }

    /// World (y up) -> picture (y down).
    fn p(&self, w: Vec2) -> Vec2 {
        Vec2::new((w.x - self.min.x) * self.scale, (self.min.y - w.y) * self.scale)
    }

    fn len(&self, l: f32) -> f32 {
        l * self.scale
    }
}

fn ship_color(side: usize, morale: Morale) -> Rgb {
    match morale {
        Morale::Steady => C_SIDE[side],
        Morale::Shaken => C_GOLD,
        Morale::Routed => C_ROUTED,
    }
}

fn draw_ship(c: &mut Canvas, cam: &Cam, pos: Vec2, heading: f32, col: Rgb) {
    let k = (cam.scale * 1.4).clamp(1.0, 2.2);
    let rot = Vec2::from_angle(heading);
    let pts: Vec<Vec2> = SHIP_DART.iter().map(|v| cam.p(pos + rot.rotate(*v * k / cam.scale))).collect();
    c.polygon_fill(&pts, col, 1.0);
}

/// A horizontal bar `frac` full at picture position `at`.
fn bar(c: &mut Canvas, at: Vec2, w: f32, frac: f32, col: Rgb) {
    let (x0, y0) = (at.x, at.y);
    c.polygon_fill(&[Vec2::new(x0, y0), Vec2::new(x0 + w, y0), Vec2::new(x0 + w, y0 + 5.0), Vec2::new(x0, y0 + 5.0)], C_GRID, 0.8);
    let f = w * frac.clamp(0.0, 1.0);
    if f > 0.5 {
        c.polygon_fill(&[Vec2::new(x0, y0), Vec2::new(x0 + f, y0), Vec2::new(x0 + f, y0 + 5.0), Vec2::new(x0, y0 + 5.0)], col, 1.0);
    }
}

fn render(card: &mut Card) -> Canvas {
    let demo = card.demo;
    let cam = Cam::of(demo);
    let mut c = Canvas::new(IMG_W, IMG_H, C_BG);
    let w = &mut card.world;
    let divs: Vec<(Entity, Division)> = w.query::<(Entity, &Division)>().iter(w).map(|(e, d)| (e, d.clone())).collect();
    let morale_of = |e: Entity| divs.iter().find(|(x, _)| *x == e).map_or(Morale::Steady, |(_, d)| d.morale);
    let ships: Vec<(Vec2, f32, usize, Entity, Vec2)> =
        w.query::<(&Pos, &Heading, &Side, &InDivision, &Slot)>().iter(w).map(|(p, h, s, d, sl)| (p.0, h.0, s.0, d.0, sl.0)).collect();
    let hash = w.resource::<SpatialHash>();
    let battle = w.resource::<Battle>().clone();
    let tuning = w.resource::<Tuning>().clone();
    let time = battle.time;

    // Underlay: what the system works on.
    match demo {
        Demo::RebuildHash => {
            let cell = hash.cell;
            let mut counts: std::collections::HashMap<(i32, i32), u32> = std::collections::HashMap::new();
            for e in &hash.entries {
                *counts.entry(((e.pos.x / cell).floor() as i32, (e.pos.y / cell).floor() as i32)).or_default() += 1;
            }
            for ((cx, cy), n) in &counts {
                let a = cam.p(Vec2::new(*cx as f32 * cell, (*cy + 1) as f32 * cell));
                let b = cam.p(Vec2::new((*cx + 1) as f32 * cell, *cy as f32 * cell));
                let pts = [a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y)];
                c.polygon_fill(&pts, C_GOLD, (0.04 + *n as f32 * 0.012).min(0.3));
            }
            let (lo, hi) = ((cam.min.x / cell).floor() as i32, ((cam.min.x + IMG_W as f32 / cam.scale) / cell).ceil() as i32);
            for k in lo..=hi {
                let x = cam.p(Vec2::new(k as f32 * cell, 0.0)).x;
                c.line(Vec2::new(x, 0.0), Vec2::new(x, IMG_H as f32), 1.0, C_GRID, 0.9);
            }
            let top = cam.min.y;
            let (lo, hi) = (((top - IMG_H as f32 / cam.scale) / cell).floor() as i32, (top / cell).ceil() as i32);
            for k in lo..=hi {
                let y = cam.p(Vec2::new(0.0, k as f32 * cell)).y;
                c.line(Vec2::new(0.0, y), Vec2::new(IMG_W as f32, y), 1.0, C_GRID, 0.9);
            }
            // The probe query.
            let probe = Vec2::new(-20.0, 0.0) + Vec2::from_angle(time * 0.35) * Vec2::new(230.0, 120.0);
            let r = 90.0;
            c.circle(cam.p(probe), cam.len(r), C_GREEN, 0.9);
            hash.for_each_within(probe, r, |e, _| c.circle(cam.p(e.pos), 4.5, C_GREEN, 1.0));
            for d in hash.divisions.values() {
                let p = cam.p(d.centroid);
                c.line(p - Vec2::new(6.0, 0.0), p + Vec2::new(6.0, 0.0), 2.0, C_TEXT, 1.0);
                c.line(p - Vec2::new(0.0, 6.0), p + Vec2::new(0.0, 6.0), 2.0, C_TEXT, 1.0);
            }
        }
        Demo::Integrate => {
            for (p, _, side, _, _) in &ships {
                if *side == 1 {
                    c.circle(cam.p(*p), cam.len(tuning.stop_distance.max(tuning.spacing * 1.4)), C_SIDE[1], 0.07);
                }
            }
        }
        Demo::RemoveFled => {
            let b = battle.bounds;
            let pts = [cam.p(Vec2::new(b.min.x, b.max.y)), cam.p(b.max), cam.p(Vec2::new(b.max.x, b.min.y)), cam.p(b.min)];
            c.polygon_stroke(&pts, 1.5, C_DIM, 0.9);
        }
        Demo::CloseRanks => {
            // Empty slots: the layout's slots no live ship holds.
            for (e, d) in &divs {
                let all = formation::layout(d.formation, d.slots as usize, tuning.spacing);
                let held: Vec<Vec2> = ships.iter().filter(|s| s.3 == *e).map(|s| s.4).collect();
                for s in all {
                    if !held.iter().any(|h| h.distance(s) < 1.0) {
                        c.circle(cam.p(d.to_world(s)), cam.len(3.5), C_DIM, 0.8);
                    }
                }
            }
        }
        Demo::SteerShips => {
            for (p, _, _, div, slot) in &ships {
                if let Some((_, d)) = divs.iter().find(|(e, _)| e == div) {
                    let target = d.to_world(*slot);
                    c.disc(cam.p(target), 2.0, C_DIM, 0.9);
                    c.line(cam.p(*p), cam.p(target), 1.0, C_GOLD, 0.45);
                }
            }
        }
        _ => {}
    }

    // Shots.
    for (from, to, side, hit, age) in &card.shots {
        let life = if *hit { LASER_HIT_SECS } else { LASER_MISS_SECS };
        let a = (1.0 - age / life).clamp(0.0, 1.0) * if *hit { 0.95 } else { 0.35 };
        c.line(cam.p(*from), cam.p(*to), if *hit { 1.5 } else { 1.0 }, C_SIDE[*side], a);
    }
    // Ships.
    for (p, h, side, div, _) in &ships {
        draw_ship(&mut c, &cam, *p, *h, ship_color(*side, morale_of(*div)));
    }
    for (p, age) in &card.booms {
        let k = age / BOOM_SECS;
        c.circle(cam.p(*p), 3.0 + 9.0 * k, C_GOLD, (1.0 - k) * 0.9);
    }

    // Overlay: what the system wrote.
    match demo {
        Demo::TickClock => {
            let ticks = ((time / DT).round() as u32) % 20;
            for k in 0..20 {
                let x = 12.0 + k as f32 * 14.0;
                let col = if k < ticks || (ticks == 0 && k == 0 && time > 0.0) { C_GREEN } else { C_GRID };
                c.polygon_fill(&[Vec2::new(x, 12.0), Vec2::new(x + 10.0, 12.0), Vec2::new(x + 10.0, 24.0), Vec2::new(x, 24.0)], col, 1.0);
            }
            // A second hand.
            let centre = Vec2::new(IMG_W as f32 - 34.0, 34.0);
            c.circle(centre, 22.0, C_DIM, 0.9);
            let hand = Vec2::from_angle(std::f32::consts::FRAC_PI_2 - time.fract() * std::f32::consts::TAU);
            c.line(centre, centre + Vec2::new(hand.x, -hand.y) * 19.0, 2.0, C_GOLD, 1.0);
        }
        Demo::Commander => {
            for (e, d) in &divs {
                let from = hash.centroid(*e).unwrap_or(d.anchor);
                match d.order {
                    Order::Engage(t) => {
                        if let Some(tc) = hash.centroid(t) {
                            c.arrow(cam.p(from), cam.p(from.lerp(tc, 0.85)), C_SIDE[d.side], 0.95);
                        }
                    }
                    Order::MoveTo { to, .. } => {
                        c.dashed_line(cam.p(from), cam.p(to), 5.0, 4.0, C_SIDE[d.side], 0.8);
                        c.circle(cam.p(to), 4.0, C_SIDE[d.side], 0.8);
                    }
                    _ => {}
                }
            }
        }
        Demo::SteerDivisions | Demo::SteerShips => {
            for (_, d) in divs.iter().filter(|(_, d)| d.side == 0) {
                let a = cam.p(d.anchor);
                c.line(a - Vec2::new(7.0, 0.0), a + Vec2::new(7.0, 0.0), 2.0, C_TEXT, 1.0);
                c.line(a - Vec2::new(0.0, 7.0), a + Vec2::new(0.0, 7.0), 2.0, C_TEXT, 1.0);
                c.arrow(a, cam.p(d.anchor + d.forward() * 70.0), C_TEXT, 0.9);
                if let Order::MoveTo { to, facing } = d.order
                    && demo == Demo::SteerDivisions
                {
                    c.circle(cam.p(to), 9.0, C_GOLD, 0.9);
                    c.arrow(cam.p(to), cam.p(to + Vec2::from_angle(facing) * 50.0), C_GOLD, 0.8);
                    c.dashed_line(a, cam.p(to), 4.0, 4.0, C_GOLD, 0.5);
                }
            }
        }
        Demo::Fire => {
            if let Some((p, h, _, _, _)) = ships.iter().filter(|s| s.2 == 0).min_by(|a, b| a.0.y.abs().total_cmp(&b.0.y.abs())) {
                for side in [-1.0, 1.0] {
                    let edge = *p + Vec2::from_angle(h + side * tuning.fire_arc) * tuning.range;
                    c.line(cam.p(*p), cam.p(edge), 1.0, C_GREEN, 0.6);
                }
                let n = 16;
                let arc: Vec<Vec2> = (0..=n)
                    .map(|k| cam.p(*p + Vec2::from_angle(h - tuning.fire_arc + 2.0 * tuning.fire_arc * k as f32 / n as f32) * tuning.range))
                    .collect();
                for k in 0..n {
                    c.line(arc[k], arc[k + 1], 1.0, C_GREEN, 0.6);
                }
                c.circle(cam.p(*p), 6.0, C_GREEN, 1.0);
            }
        }
        Demo::Morale => {
            for (e, d) in &divs {
                if let Some(cn) = hash.centroid(*e) {
                    let at = cam.p(cn) + Vec2::new(-30.0, cam.len(d.half_width) + 12.0);
                    bar(&mut c, at, 60.0, d.shock * tuning.k_shock / tuning.rout_at, ship_color(d.side, d.morale));
                }
            }
        }
        Demo::CheckEnd => {
            let lost = battle.lost;
            for side in 0..2 {
                let start = battle.start[side].max(1) as f32;
                let fighting = (start - lost[side] as f32) / start;
                bar(&mut c, Vec2::new(12.0, 12.0 + side as f32 * 10.0), 200.0, fighting, C_SIDE[side]);
            }
            if let Some(o) = &battle.result {
                let col = o.winner.map_or(C_TEXT, |s| C_SIDE[s]);
                let (w, h) = (IMG_W as f32, IMG_H as f32);
                c.polygon_stroke(&[Vec2::new(2.0, 2.0), Vec2::new(w - 2.0, 2.0), Vec2::new(w - 2.0, h - 2.0), Vec2::new(2.0, h - 2.0)], 4.0, col, 1.0);
            }
        }
        _ => {}
    }
    c
}

fn status(card: &mut Card) -> String {
    let w = &mut card.world;
    let b = w.resource::<Battle>().clone();
    let t = format!("t = {:.2} s", b.time);
    let divs: Vec<Division> = w.query::<&Division>().iter(w).cloned().collect();
    let mut sorted = divs.clone();
    sorted.sort_by_key(|d| (d.side, d.number));
    match card.demo {
        Demo::TickClock => format!("{t} · {} ticks of {DT} s", (b.time / DT).round() as u32),
        Demo::RebuildHash => {
            let h = w.resource::<SpatialHash>();
            format!("{t} · {} ships in the hash · {} division centroids", h.entries.len(), h.divisions.len())
        }
        Demo::Commander => {
            let orders: Vec<String> = sorted.iter().filter(|d| d.alive > 0).map(|d| format!("{} {}", d.label(), d.order.name())).collect();
            format!("{t} · next think in {:.1} s · {}", (2.0 - b.ai_timer).max(0.0), orders.join(", "))
        }
        Demo::SteerDivisions | Demo::SteerShips => {
            let d = sorted.iter().find(|d| d.side == 0);
            d.map_or(t.clone(), |d| {
                format!("{t} · {} {} · {} · facing {:.0}°", d.label(), d.formation.name(), d.order.name(), d.facing.to_degrees().rem_euclid(360.0))
            })
        }
        Demo::Integrate => {
            let closest = sorted.iter().find(|d| d.side == 0).map_or(0.0, |d| d.closest);
            format!("{t} · Blue's nearest ship to an enemy: {closest:.0} (blocking distance {:.1})", w.resource::<Tuning>().stop_distance)
        }
        Demo::Fire => {
            let shots = card.shots.len();
            format!("{t} · destroyed: Blue {} · Red {} · shots on screen {shots}", b.destroyed[0], b.destroyed[1])
        }
        Demo::RemoveFled => format!("{t} · Blue fled: {} · left in the box: {}", b.fled[0], sorted.first().map_or(0, |d| d.alive)),
        Demo::CloseRanks => {
            let s: Vec<String> = sorted.iter().map(|d| format!("{} {} {}/{} slots", d.label(), d.formation.name(), d.alive, d.slots)).collect();
            let next = 3.0 - b.time % 3.0;
            format!("{t} · re-form in {next:.1} s · {}", s.join(", "))
        }
        Demo::Morale => {
            let s: Vec<String> = sorted.iter().map(|d| format!("{} {} (shock {:.2})", d.label(), d.morale.name(), d.shock)).collect();
            format!("{t} · {}", s.join(", "))
        }
        Demo::CheckEnd => match &b.result {
            Some(o) => {
                let who = o.winner.map_or("Draw".to_string(), |s| format!("{} wins", side_name(s)));
                let why = match o.reason {
                    EndReason::Broke => "a side broke",
                    EndReason::Disengaged => "disengaged",
                    EndReason::TimeLimit => "time limit",
                };
                format!("{t} · OVER: {who} ({why}) · restarting…")
            }
            None => format!(
                "{t} · fighting: Blue {} · Red {} · lost: Blue {} · Red {}",
                b.start[0] - b.lost[0].min(b.start[0]),
                b.start[1] - b.lost[1].min(b.start[1]),
                b.lost[0],
                b.lost[1]
            ),
        },
    }
}

// --- Page and systems ---

pub fn setup_ecs_labs(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let cards: Vec<Card> = Demo::ALL.iter().map(|d| Card::new(*d, images.add(blank_image()), 0)).collect();
    let handles: Vec<Handle<Image>> = cards.iter().map(|c| c.image.clone()).collect();
    commands.insert_resource(EcsLabs { cards, paused: false, speed: 1, acc: 0.0 });

    let root = Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        overflow: Overflow::scroll_y(),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        display: Display::None,
        ..default()
    };
    commands.spawn((EcsPage, root, BackgroundColor(BG), ScrollPosition::default(), GlobalZIndex(10))).with_children(|m| {
        m.spawn(Node {
            flex_direction: FlexDirection::Column,
            width: Val::Percent(100.0),
            max_width: Val::Px(1240.0),
            padding: UiRect::all(Val::Px(16.0)),
            row_gap: Val::Px(4.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|c| {
            c.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(16.0), ..row() }).with_children(|r| {
                button_for(r, "← Home", super::lab_ui::LabAction::Back, BtnKind::Normal);
                r.spawn(text("ECS systems", 24.0, TEXT));
            });
            c.spawn((
                text(
                    "The battle is a Bevy ECS: every ship is an entity (Ship, Side, Pos, Vel, Heading, InDivision, Slot, Weapon), \
                     every division is an entity its ships point at (Division), and the SimTick schedule runs these systems in \
                     order, 20 times a battle-second. Each card below runs its own small World with that same schedule, set up so \
                     one system does the visible work.",
                    14.0,
                    DIM,
                ),
                Node { margin: UiRect::bottom(Val::Px(8.0)), ..default() },
            ));
            c.spawn(panel()).with_children(|p| {
                p.spawn(row()).with_children(|r| {
                    button_for(r, "Pause", EcsAction::Pause, BtnKind::Primary);
                    for m in [1, 2, 4] {
                        button_for(r, &format!("{m}×"), EcsAction::Speed(m), BtnKind::Choice);
                    }
                    button_for(r, "Restart all", EcsAction::RestartAll, BtnKind::Normal);
                });
                let order: Vec<&str> = Demo::ALL.iter().map(|d| d.name()).collect();
                p.spawn(text(format!("SimTick: {}", order.join(" → ")), 12.5, GOLD));
            });
            c.spawn(Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(12.0), ..default() })
                .with_children(|g| {
                    for (i, demo) in Demo::ALL.iter().enumerate() {
                        g.spawn(panel())
                            .insert(Node {
                                flex_direction: FlexDirection::Column,
                                padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                                margin: UiRect::bottom(Val::Px(12.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(8.0)),
                                row_gap: Val::Px(6.0),
                                width: Val::Percent(49.0),
                                min_width: Val::Px(360.0),
                                flex_grow: 1.0,
                                ..default()
                            })
                            .with_children(|p| {
                                p.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(10.0), ..row() }).with_children(|r| {
                                    r.spawn(text(format!("{}. {}", i + 1, demo.name()), 17.0, TEXT));
                                    button_for(r, "Restart", EcsAction::Restart(i), BtnKind::Normal);
                                });
                                card_title(p, "", demo.blurb());
                                p.spawn((EcsStatus(i), text("", 12.5, GREEN)));
                                p.spawn((
                                    ImageNode::new(handles[i].clone()),
                                    Node {
                                        width: Val::Percent(100.0),
                                        aspect_ratio: Some(IMG_W as f32 / IMG_H as f32),
                                        flex_shrink: 0.0,
                                        ..default()
                                    },
                                ));
                            });
                    }
                });
        });
    });
}

fn on_page(phase: &Phase) -> bool {
    *phase == Phase::Ecs
}

pub fn handle_ecs_buttons(buttons: Query<(&Interaction, &EcsAction), Changed<Interaction>>, mut labs: ResMut<EcsLabs>) {
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *action {
            EcsAction::Pause => labs.paused = !labs.paused,
            EcsAction::Speed(m) => {
                labs.speed = m;
                labs.paused = false;
            }
            EcsAction::Restart(i) => labs.cards[i].restart(),
            EcsAction::RestartAll => labs.cards.iter_mut().for_each(Card::restart),
        }
    }
}

/// Steps every card's world at the chosen speed while the page is open.
pub fn step_ecs_labs(time: Res<Time>, phase: Res<Phase>, mut labs: ResMut<EcsLabs>) {
    if !on_page(&phase) || labs.paused {
        return;
    }
    labs.acc += time.delta_secs().min(0.1) * labs.speed as f32;
    let ticks = ((labs.acc / DT) as u32).min(MAX_TICKS * labs.speed);
    if ticks == 0 {
        return;
    }
    labs.acc = (labs.acc - ticks as f32 * DT).max(0.0);
    for card in &mut labs.cards {
        card.tick(ticks);
    }
}

/// Redraws the pictures and status lines, and shows the page only when open.
pub fn render_ecs_labs(
    phase: Res<Phase>,
    mut labs: ResMut<EcsLabs>,
    mut images: ResMut<Assets<Image>>,
    mut page: Query<&mut Node, With<EcsPage>>,
    mut texts: Query<(&EcsStatus, &mut Text)>,
    mut since: Local<f32>,
    time: Res<Time>,
) {
    let open = on_page(&phase);
    for mut n in &mut page {
        let d = if open { Display::Flex } else { Display::None };
        if n.display != d {
            n.display = d;
        }
    }
    if !open {
        return;
    }
    *since += time.delta_secs();
    if *since < 1.0 / 30.0 {
        return;
    }
    *since = 0.0;
    for card in &mut labs.cards {
        let rgba = render(card).to_rgba();
        if let Some(mut img) = images.get_mut(&card.image) {
            img.data = Some(rgba);
        }
    }
    for (s, mut t) in &mut texts {
        let line = status(&mut labs.cards[s.0]);
        if t.0 != line {
            t.0 = line;
        }
    }
}

pub fn ecs_button_state(labs: Res<EcsLabs>, mut buttons: Query<(&EcsAction, &mut Selected, &mut Enabled, &BtnLabel)>, mut texts: Query<&mut Text>) {
    for (action, mut sel, mut enabled, labels) in &mut buttons {
        if !enabled.0 {
            enabled.0 = true;
        }
        let on = match *action {
            EcsAction::Pause => labs.paused,
            EcsAction::Speed(m) => !labs.paused && labs.speed == m,
            _ => false,
        };
        if sel.0 != on {
            sel.0 = on;
        }
        if *action == EcsAction::Pause {
            let label = if labs.paused { "▶ Resume" } else { "Pause" };
            if let Some(e) = labels.first()
                && let Ok(mut t) = texts.get_mut(e)
                && t.0 != label
            {
                t.0 = label.to_string();
            }
        }
    }
}

/// Wheel scrolling for the page.
pub fn scroll_ecs_labs(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    phase: Res<Phase>,
    mut page: Query<&mut ScrollPosition, With<EcsPage>>,
) {
    let dy: f32 = wheel
        .read()
        .map(|ev| match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y * 28.0,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    if dy == 0.0 || !on_page(&phase) {
        return;
    }
    if let Ok(mut s) = page.single_mut() {
        s.0.y = (s.0.y - dy).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every card runs without panicking and its battle keeps going (or
    /// restarts) for a minute of battle time.
    #[test]
    fn cards_run() {
        for demo in Demo::ALL {
            let mut card = Card::new(demo, Handle::default(), 0);
            for k in 0..60 {
                card.tick(20);
                if let Ok(dir) = std::env::var("SG_ECS_SHOTS")
                    && k % 10 == 9
                {
                    let path = format!("{dir}/{:02}-{}-{k:02}.ppm", demo as usize, demo.name().replace("::", "_"));
                    std::fs::write(path, render(&mut card).to_ppm()).unwrap();
                    println!("{} @{k}: {}", demo.name(), status(&mut card));
                }
            }
            let _ = status(&mut card);
        }
    }
}
