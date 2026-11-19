//! Fleet layouts and scenarios: where each division starts, in what shape,
//! and with what opening order.

use bevy::prelude::*;

use super::battle::{Battle, Force, MIN_FORCE, SideMods, SimSettings, Tuning};
use super::components::*;
use super::formation::{self, Formation};
use super::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FleetShape {
    #[default]
    Line,
    Arrow,
    Crescent,
    Echelon,
    Columns,
}

impl FleetShape {
    pub const ALL: [FleetShape; 5] = [FleetShape::Line, FleetShape::Arrow, FleetShape::Crescent, FleetShape::Echelon, FleetShape::Columns];

    pub fn name(self) -> &'static str {
        match self {
            FleetShape::Line => "Line",
            FleetShape::Arrow => "Arrow",
            FleetShape::Crescent => "Crescent",
            FleetShape::Echelon => "Echelon",
            FleetShape::Columns => "Columns",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Scenario {
    pub title: &'static str,
    pub blurb: &'static str,
    pub fleets: [FleetShape; 2],
    /// Side 1 fights on low supply (slower fire, shakier morale).
    pub low_supply: bool,
}

pub const SCENARIOS: [Scenario; 6] = [
    Scenario {
        title: "Arrow vs Line",
        blurb: "A wedge-tipped arrow drives at the centre of a wide line.",
        fleets: [FleetShape::Arrow, FleetShape::Line],
        low_supply: false,
    },
    Scenario {
        title: "Line vs Line",
        blurb: "Two lines meet head on: the even fight.",
        fleets: [FleetShape::Line, FleetShape::Line],
        low_supply: false,
    },
    Scenario {
        title: "Crescent vs Arrow",
        blurb: "The horns of a crescent close on an arrow's flanks.",
        fleets: [FleetShape::Crescent, FleetShape::Arrow],
        low_supply: false,
    },
    Scenario {
        title: "Echelon vs Line",
        blurb: "A refused flank: one wing strikes first, the other hangs back.",
        fleets: [FleetShape::Echelon, FleetShape::Line],
        low_supply: false,
    },
    Scenario {
        title: "Columns vs Line",
        blurb: "Deep columns against a line: depth pushes, width shoots.",
        fleets: [FleetShape::Columns, FleetShape::Line],
        low_supply: false,
    },
    Scenario {
        title: "Low supply",
        blurb: "Line against line, Red short of munitions.",
        fleets: [FleetShape::Line, FleetShape::Line],
        low_supply: true,
    },
];

/// One division's starting place in its fleet's frame (+x toward the enemy).
#[derive(Clone, Copy, Debug)]
pub struct DivisionSpec {
    pub formation: Formation,
    pub offset: Vec2,
    pub facing: f32,
    pub ships: usize,
    pub order: Order,
    pub flagship: bool,
}

/// Five fighting divisions plus a flagship reserve.
pub fn fleet_layout(shape: FleetShape, ships: usize, spacing: f32) -> Vec<DivisionSpec> {
    let reserve = ships / 6;
    let line_total = ships - reserve;
    let sizes: Vec<usize> = (0..5).map(|i| line_total / 5 + usize::from(i < line_total % 5)).collect();
    let d = sizes[0];
    let line_w = formation::frontage(Formation::Line, d, spacing);
    let gap = spacing * 2.0;
    let spec = |formation, offset, facing, ships, order| DivisionSpec { formation, offset, facing, ships, order, flagship: false };
    let mut divs: Vec<DivisionSpec> = match shape {
        FleetShape::Line => (0..5)
            .map(|i| spec(Formation::Line, Vec2::new(0.0, (i as f32 - 2.0) * (line_w + gap)), 0.0, sizes[i], Order::Advance))
            .collect(),
        FleetShape::Arrow => {
            // The whole fleet on a narrow front: a wedge tip, swept-back
            // flanks hugging it, and two columns as the shaft behind.
            let tip = formation::layout(Formation::Wedge, sizes[2], spacing);
            let (tip_back, tip_half) = formation::extent(&tip);
            let sweep = 0.5f32;
            let flank_w = formation::frontage(Formation::Line, sizes[1], spacing);
            let (wy, wx) = (flank_w / 2.0 * sweep.cos(), flank_w / 2.0 * sweep.sin());
            let col_w = formation::frontage(Formation::Column, sizes[0], spacing);
            let col_depth = -formation::extent(&formation::layout(Formation::Column, sizes[0], spacing)).0;
            let flank_x = tip_back * 0.5 - wx;
            let shaft_x = tip_back - col_depth - 2.0 * gap;
            vec![
                spec(Formation::Column, Vec2::new(shaft_x, col_w / 2.0 + gap), 0.0, sizes[0], Order::Charge),
                spec(Formation::EchelonLeft, Vec2::new(flank_x, tip_half + wy + gap * 0.5), 0.0, sizes[1], Order::Charge),
                spec(Formation::Wedge, Vec2::ZERO, 0.0, sizes[2], Order::Charge),
                spec(Formation::EchelonRight, Vec2::new(flank_x, -(tip_half + wy + gap * 0.5)), 0.0, sizes[3], Order::Charge),
                spec(Formation::Column, Vec2::new(shaft_x, -(col_w / 2.0 + gap)), 0.0, sizes[4], Order::Charge),
            ]
        }
        FleetShape::Crescent => (0..5)
            .map(|i| {
                let t = i as f32 - 2.0;
                let a = t * 0.38;
                let r = 2.6 * line_w;
                let offset = Vec2::new(r * (1.0 - a.cos()) - 60.0, r * a.sin());
                // The horns face slightly inward to close on the enemy.
                spec(Formation::Line, offset, -a * 0.5, sizes[i], Order::Advance)
            })
            .collect(),
        FleetShape::Echelon => (0..5)
            .map(|i| {
                let offset = Vec2::new(-((4 - i) as f32) * 90.0, (i as f32 - 2.0) * (line_w + gap));
                spec(Formation::Line, offset, 0.0, sizes[i], Order::Advance)
            })
            .collect(),
        FleetShape::Columns => {
            let col_w = formation::frontage(Formation::Column, d, spacing);
            (0..5)
                .map(|i| spec(Formation::Column, Vec2::new(0.0, (i as f32 - 2.0) * (col_w + 9.0 * spacing)), 0.0, sizes[i], Order::Advance))
                .collect()
        }
    };
    let back = divs.iter().map(|d| d.offset.x).fold(0.0, f32::min);
    divs.push(DivisionSpec {
        formation: Formation::Line,
        offset: Vec2::new(back - 220.0, 0.0),
        facing: 0.0,
        ships: reserve,
        order: Order::Advance,
        flagship: true,
    });
    divs
}

/// Fleet centres start this far from the middle of the battle area.
pub const START_X: f32 = 650.0;

pub fn home_facing(side: usize) -> f32 {
    if side == 0 { 0.0 } else { std::f32::consts::PI }
}

/// Spawns both fleets for the current settings and resets the battle.
pub fn spawn_battle(world: &mut World) {
    // Clear any previous battle.
    let old: Vec<Entity> =
        world.query_filtered::<Entity, Or<(With<Ship>, With<Division>)>>().iter(world).collect();
    for e in old {
        world.despawn(e);
    }
    let settings = *world.resource::<SimSettings>();
    let tuning = world.resource::<Tuning>().clone();
    let scenario = SCENARIOS[settings.scenario.min(SCENARIOS.len() - 1)];
    let low_supply_side = if settings.mirror { 0 } else { 1 };
    // Each side's ships, damaged ships, shape and modifiers: from the
    // campaign forces when given, otherwise from the scenario.
    let forces: [Force; 2] = match settings.forces {
        Some(f) => f.map(|f| Force { ships: f.ships.max(MIN_FORCE), damaged: f.damaged.min(f.ships), ..f }),
        None => {
            let shapes = if settings.mirror { [scenario.fleets[1], scenario.fleets[0]] } else { scenario.fleets };
            std::array::from_fn(|side| {
                let mut mods = SideMods::default();
                if scenario.low_supply && side == low_supply_side {
                    mods.fire_rate = tuning.low_supply_fire;
                    mods.morale = tuning.k_supply;
                }
                Force { ships: settings.ships as u32, damaged: 0, shape: shapes[side], mods }
            })
        }
    };

    let mut widest: f32 = 0.0;
    let mut start = [0u32; 2];
    for side in 0..2 {
        let facing = home_facing(side);
        let centre = Vec2::new(if side == 0 { -START_X } else { START_X }, 0.0);
        let rot = Vec2::from_angle(facing);
        let layout = fleet_layout(forces[side].shape, forces[side].ships as usize, tuning.spacing);
        // Damaged ships are spread evenly through the fleet.
        let (total, damaged) = (forces[side].ships, forces[side].damaged);
        let mut spawned = 0u32;
        for (i, spec) in layout.iter().enumerate() {
            let slots = formation::layout(spec.formation, spec.ships, tuning.spacing);
            let (depth, half_width) = formation::extent(&slots);
            let anchor = centre + rot.rotate(spec.offset);
            widest = widest.max(anchor.y.abs() + half_width);
            let div_facing = facing + if side == 0 { spec.facing } else { -spec.facing };
            let division = world
                .spawn(Division {
                    side,
                    number: i + 1,
                    formation: spec.formation,
                    anchor,
                    facing: div_facing,
                    order: spec.order,
                    morale: Morale::Steady,
                    start: spec.ships as u32,
                    alive: spec.ships as u32,
                    slots: spec.ships as u32,
                    shock: 0.0,
                    flagship: spec.flagship,
                    depth,
                    half_width,
                    since_contact: f32::INFINITY,
                    closest: f32::INFINITY,
                })
                .id();
            let div_rot = Vec2::from_angle(div_facing);
            // The flagship takes the slot nearest the division's centre.
            let flag_slot = spec.flagship.then(|| {
                (0..slots.len()).min_by(|a, b| slots[*a].length_squared().total_cmp(&slots[*b].length_squared())).unwrap_or(0)
            });
            for (k, slot) in slots.iter().enumerate() {
                let (jx, jy, cd) = {
                    let mut rng = world.resource_mut::<Rng>();
                    (rng.range(-1.5, 1.5), rng.range(-1.5, 1.5), rng.range(0.0, tuning.cooldown))
                };
                let flagship = flag_slot == Some(k);
                let max_hull = if flagship { tuning.flagship_hull } else { tuning.hull };
                let is_damaged = !flagship && (spawned as u64 * damaged as u64 / total.max(1) as u64) != ((spawned as u64 + 1) * damaged as u64 / total.max(1) as u64);
                spawned += 1;
                let hull = if is_damaged { (max_hull * 0.5).ceil() } else { max_hull };
                let mut ship = world.spawn((
                    Ship { hull, max_hull },
                    Side(side),
                    Pos(anchor + div_rot.rotate(*slot) + Vec2::new(jx, jy)),
                    Vel::default(),
                    Heading(div_facing),
                    InDivision(division),
                    Slot(*slot),
                    Weapon { cooldown: cd, aim: None },
                ));
                if flagship {
                    ship.insert(FleetFlagship);
                }
            }
            start[side] += spec.ships as u32;
        }
    }

    let half_h = (widest + 500.0).max(1300.0);
    *world.resource_mut::<Battle>() = Battle {
        start,
        mods: forces.map(|f| f.mods),
        shapes: forces.map(|f| f.shape),
        bounds: Rect::new(-1800.0, -half_h, 1800.0, half_h),
        ..default()
    };
}
