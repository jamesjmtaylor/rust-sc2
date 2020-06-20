#[macro_use]
extern crate clap;

use rand::prelude::{thread_rng, SliceRandom};
use rust_sc2::prelude::*;
use std::cmp::Ordering;

#[bot]
#[derive(Default)]
struct ZergRushAI {
	last_loop_distributed: u32,
}

impl ZergRushAI {
	const DISTRIBUTION_DELAY: u32 = 8;

	fn new() -> Self {
		Default::default()
	}
	fn distribute_workers(&mut self) {
		if self.units.my.workers.is_empty() {
			return;
		}
		let mut idle_workers = self.units.my.workers.idle();
		let bases = self.units.my.townhalls.ready();

		// Check distribution delay if there aren't any idle workers
		let game_loop = self.state.observation.game_loop;
		let last_loop = &mut self.last_loop_distributed;
		if idle_workers.is_empty() && *last_loop + Self::DISTRIBUTION_DELAY + bases.len() as u32 > game_loop {
			return;
		}
		*last_loop = game_loop;

		// Distribute
		let mineral_fields = &self.units.mineral_fields;
		if mineral_fields.is_empty() {
			return;
		}
		if bases.is_empty() {
			return;
		}
		let gas_buildings = self.units.my.gas_buildings.ready();

		let mut deficit_minings = Units::new();
		let mut deficit_geysers = Units::new();

		let mineral_tags = mineral_fields.iter().map(|m| m.tag).collect::<Vec<u64>>();

		let speed_upgrade = UpgradeId::Zerglingmovementspeed;
		let has_enough_gas = self.can_afford_upgrade(speed_upgrade)
			|| self.has_upgrade(speed_upgrade)
			|| self.is_ordered_upgrade(speed_upgrade);

		bases.iter().for_each(
			|base| match base.assigned_harvesters.cmp(&base.ideal_harvesters) {
				Ordering::Equal => {}
				Ordering::Greater => {
					let local_minerals = self
						.units
						.mineral_fields
						.closer(11.0, base)
						.iter()
						.map(|m| m.tag)
						.collect::<Vec<u64>>();

					idle_workers.extend(
						self.units
							.my
							.workers
							.filter(|u| {
								u.target_tag().map_or(false, |target_tag| {
									local_minerals.contains(&target_tag)
										|| (u.is_carrying_minerals() && target_tag == base.tag)
								})
							})
							.iter()
							.take(
								(base.assigned_harvesters.unwrap() - base.ideal_harvesters.unwrap()) as usize,
							)
							.cloned(),
					);
				}
				Ordering::Less => (0..(base.ideal_harvesters.unwrap() - base.assigned_harvesters.unwrap()))
					.for_each(|_| {
						deficit_minings.push(base.clone());
					}),
			},
		);

		if has_enough_gas {
			gas_buildings.iter().for_each(|gas| {
				if let Ordering::Greater = gas.assigned_harvesters.cmp(&Some(0)) {
					idle_workers.extend(
						self.units
							.my
							.workers
							.filter(|u| {
								u.target_tag().map_or(false, |target_tag| {
									target_tag == gas.tag
										|| (u.is_carrying_vespene()
											&& target_tag == bases.closest(gas).unwrap().tag)
								})
							})
							.iter()
							.cloned(),
					);
				}
			});
		} else {
			gas_buildings
				.iter()
				.for_each(|gas| match gas.assigned_harvesters.cmp(&gas.ideal_harvesters) {
					Ordering::Equal => {}
					Ordering::Greater => {
						idle_workers.extend(
							self.units
								.my
								.workers
								.filter(|u| {
									u.target_tag().map_or(false, |target_tag| {
										target_tag == gas.tag
											|| (u.is_carrying_vespene()
												&& target_tag == bases.closest(gas).unwrap().tag)
									})
								})
								.iter()
								.take(
									(gas.assigned_harvesters.unwrap() - gas.ideal_harvesters.unwrap())
										as usize,
								)
								.cloned(),
						);
					}
					Ordering::Less => {
						idle_workers.extend(
							self.units
								.my
								.workers
								.filter(|u| {
									u.target_tag()
										.map_or(false, |target_tag| mineral_tags.contains(&target_tag))
								})
								.iter()
								.cloned(),
						);
						(0..(gas.ideal_harvesters.unwrap() - gas.assigned_harvesters.unwrap())).for_each(
							|_| {
								deficit_geysers.push(gas.clone());
							},
						);
					}
				});
		}

		let minerals_near_base = if idle_workers.len() > deficit_minings.len() + deficit_geysers.len() {
			let minerals = mineral_fields.filter(|m| bases.iter().any(|base| base.is_closer(11.0, *m)));
			if minerals.is_empty() {
				None
			} else {
				Some(minerals)
			}
		} else {
			None
		};

		let mineral_fields = mineral_fields.clone();
		idle_workers.iter().for_each(|u| {
			if !deficit_geysers.is_empty() {
				let closest = deficit_geysers.closest(u).unwrap().tag;
				deficit_geysers.remove(closest);
				u.gather(closest, false);
			} else if !deficit_minings.is_empty() {
				let closest = deficit_minings.closest(u).unwrap().clone();
				deficit_minings.remove(closest.tag);
				u.gather(
					mineral_fields
						.closer(11.0, &closest)
						.max(|m| m.mineral_contents.unwrap_or(0))
						.unwrap()
						.tag,
					false,
				);
			} else if u.is_idle() {
				if let Some(minerals) = &minerals_near_base {
					u.gather(minerals.closest(u).unwrap().tag, false);
				}
			}
		});
	}

	fn order_units(&mut self) {
		if self.units.my.larvas.is_empty() {
			return;
		}

		let over = UnitTypeId::Overlord;
		if self.supply_left < 3
			&& self.supply_cap < 200
			&& self.counter().ordered().count(over) == 0
			&& self.can_afford(over, false)
		{
			if let Some(larva) = self.units.my.larvas.pop() {
				larva.train(over, false);
				self.substract_resources(over);
			}
		}

		let drone = UnitTypeId::Drone;
		if (self.supply_workers as usize) < 96.min(self.counter().all().count(UnitTypeId::Hatchery) * 16)
			&& self.can_afford(drone, true)
		{
			if let Some(larva) = self.units.my.larvas.pop() {
				larva.train(drone, false);
				self.substract_resources(drone);
			}
		}

		let queen = UnitTypeId::Queen;
		if self.counter().all().count(queen) < self.units.my.townhalls.len() && self.can_afford(queen, true) {
			let townhalls = self.units.my.townhalls.clone();
			if !townhalls.is_empty() {
				townhalls.first().unwrap().train(queen, false);
				self.substract_resources(queen);
			}
		}

		let zergling = UnitTypeId::Zergling;
		if self.can_afford(zergling, true) {
			if let Some(larva) = self.units.my.larvas.pop() {
				larva.train(zergling, false);
				self.substract_resources(zergling);
			}
		}
	}

	fn get_builder(&self, pos: Point2, mineral_tags: &[u64]) -> Option<Unit> {
		let workers = self.units.my.workers.filter(|u| {
			!u.is_constructing()
				&& (!u.is_gathering() || u.target_tag().map_or(false, |tag| mineral_tags.contains(&tag)))
				&& !u.is_returning()
				&& !u.is_carrying_resource()
		});
		if workers.is_empty() {
			None
		} else {
			Some(workers.closest(pos).unwrap().clone())
		}
	}
	fn build(&mut self) {
		let mineral_tags = self
			.units
			.mineral_fields
			.iter()
			.map(|u| u.tag)
			.collect::<Vec<u64>>();

		let pool = UnitTypeId::SpawningPool;
		if self.counter().all().count(pool) == 0 && self.can_afford(pool, false) {
			let place = self.start_location.towards(self.game_info.map_center, 6.0);
			if let Some(location) = self.find_placement(pool, place, Default::default()) {
				if let Some(builder) = self.get_builder(location, &mineral_tags) {
					builder.build(pool, location, false);
					self.substract_resources(pool);
				}
			}
		}

		let extractor = UnitTypeId::Extractor;
		if self.counter().all().count(extractor) == 0 && self.can_afford(extractor, false) {
			let start_location = self.start_location;
			if let Some(geyser) = self.find_gas_placement(start_location) {
				if let Some(builder) = self.get_builder(geyser.position, &mineral_tags) {
					builder.build_gas(geyser.tag, false);
					self.substract_resources(extractor);
				}
			}
		}

		let hatchery = UnitTypeId::Hatchery;
		if self.can_afford(hatchery, false) {
			if let Some((location, _resource_center)) = self.get_expansion() {
				if let Some(builder) = self.get_builder(location, &mineral_tags) {
					builder.build(hatchery, location, false);
					self.substract_resources(hatchery);
				}
			}
		}
	}

	fn upgrades(&mut self) {
		let speed_upgrade = UpgradeId::Zerglingmovementspeed;
		if !self.has_upgrade(speed_upgrade)
			&& !self.is_ordered_upgrade(speed_upgrade)
			&& self.can_afford_upgrade(speed_upgrade)
		{
			let pool = self.units.my.structures.of_type(UnitTypeId::SpawningPool);
			if !pool.is_empty() {
				pool.first().unwrap().research(speed_upgrade, false);
				self.substract_upgrade_cost(speed_upgrade);
			}
		}
	}

	fn execute_micro(&mut self) {
		// Injecting Larva
		let hatcheries = self.units.my.townhalls.clone();
		if !hatcheries.is_empty() {
			let not_injected = hatcheries.filter(|h| {
				!h.has_buff(BuffId::QueenSpawnLarvaTimer)
					|| h.buff_duration_remain.unwrap() * 20 > h.buff_duration_max.unwrap()
			});
			if !not_injected.is_empty() {
				let mut queens = self.units.my.units.filter(|u| {
					u.type_id == UnitTypeId::Queen
						&& !u.is_using(AbilityId::EffectInjectLarva)
						&& u.has_ability(AbilityId::EffectInjectLarva)
				});
				for h in hatcheries.iter() {
					if queens.is_empty() {
						break;
					}
					let queen = queens.closest(h).unwrap().clone();
					queens.remove(queen.tag);
					queen.command(AbilityId::EffectInjectLarva, Target::Tag(h.tag), false);
				}
			}
		}

		let zerglings = self.units.my.units.of_type(UnitTypeId::Zergling);
		if zerglings.is_empty() {
			return;
		}

		// Check if speed upgrade is >80% ready
		let speed_upgrade = UpgradeId::Zerglingmovementspeed;
		let speed_upgrade_is_almost_ready =
			self.has_upgrade(speed_upgrade) || self.upgrade_progress(speed_upgrade) >= 0.8;

		// Attacking with zerglings or defending our locations
		let targets = {
			let enemies = if speed_upgrade_is_almost_ready {
				self.units.enemy.all.clone()
			} else {
				self.units
					.enemy
					.all
					.filter(|e| hatcheries.iter().any(|h| h.is_closer(25.0, *e)))
			};
			if enemies.is_empty() {
				None
			} else {
				let ground = enemies.ground();
				if ground.is_empty() {
					None
				} else {
					Some(ground)
				}
			}
		};
		match targets {
			Some(targets) => zerglings.iter().for_each(|u| {
				let target = {
					let close_targets = targets.in_range_of(u, 0.0);
					if !close_targets.is_empty() {
						close_targets.partial_min(|t| t.hits()).unwrap().position
					} else {
						targets.closest(u).unwrap().position
					}
				};
				u.attack(Target::Pos(target), false);
			}),
			None => {
				let target = if speed_upgrade_is_almost_ready {
					self.enemy_start
				} else {
					self.start_location.towards(self.start_center, -8.0)
				};
				zerglings.iter().for_each(|u| {
					u.move_to(Target::Pos(target), false);
				})
			}
		}
	}
}

impl Player for ZergRushAI {
	fn on_start(&mut self) -> SC2Result<()> {
		let townhall = self.units.my.townhalls.first().unwrap().clone();

		townhall.command(AbilityId::RallyWorkers, Target::Pos(self.start_center), false);
		self.units
			.my
			.larvas
			.first()
			.unwrap()
			.train(UnitTypeId::Drone, false);
		self.substract_resources(UnitTypeId::Drone);

		let minerals_near_base = self.units.mineral_fields.closer(11.0, &townhall);
		self.units.my.workers.clone().iter().for_each(|u| {
			u.gather(minerals_near_base.closest(u).unwrap().tag, false);
		});
		Ok(())
	}

	fn on_step(&mut self, _iteration: usize) -> SC2Result<()> {
		self.distribute_workers();
		self.upgrades();
		self.build();
		self.order_units();
		self.execute_micro();
		Ok(())
	}

	fn get_player_settings(&self) -> PlayerSettings {
		PlayerSettings::new(Race::Zerg, Some("RustyLings"))
	}
}

fn main() -> SC2Result<()> {
	let app = clap_app!(RustyLings =>
		(version: crate_version!())
		(author: crate_authors!())
		(@arg ladder_server: --LadderServer +takes_value)
		(@arg opponent_id: --OpponentId +takes_value)
		(@arg host_port: --GamePort +takes_value)
		(@arg player_port: --StartPort +takes_value)
		(@arg game_step: -s --step
			+takes_value
			default_value("1")
			"Sets game step for bot"
		)
		(@subcommand local =>
			(about: "Runs local game vs Computer")
			(@arg map: -m --map
				+takes_value
			)
			(@arg race: -r --race
				+takes_value
				"Sets opponent race"
			)
			(@arg difficulty: -d --difficulty
				+takes_value
				"Sets opponent diffuculty"
			)
			(@arg ai_build: --("ai-build")
				+takes_value
				"Sets opponent build"
			)
			(@arg sc2_version: --("sc2-version")
				+takes_value
				"Sets sc2 version"
			)
			(@arg save_replay: --("save-replay")
				+takes_value
				"Sets path to save replay"
			)
			(@arg realtime: --realtime "Enables realtime mode")
		)
		(@subcommand human =>
			(about: "Runs game Human vs Bot")
			(@arg map: -m --map
				+takes_value
			)
			(@arg race: -r --race *
				+takes_value
				"Sets human race"
			)
			(@arg name: --name
				+takes_value
				"Sets human name"
			)
			(@arg sc2_version: --("sc2-version")
				+takes_value
				"Sets sc2 version"
			)
			(@arg save_replay: --("save-replay")
				+takes_value
				"Sets path to save replay"
			)
		)
	)
	.get_matches();

	let game_step = match app.value_of("game_step") {
		Some("0") => panic!("game_step must be X >= 1"),
		Some(step) => step.parse::<u32>().expect("Can't parse game_step"),
		None => unreachable!(),
	};

	let mut bot = ZergRushAI::new();
	bot.game_step = game_step;

	if app.is_present("ladder_server") {
		run_ladder_game(
			&mut bot,
			app.value_of("ladder_server").unwrap_or("127.0.0.1"),
			app.value_of("host_port").expect("GamePort must be specified"),
			app.value_of("player_port")
				.expect("StartPort must be specified")
				.parse()
				.expect("Can't parse StartPort"),
			app.value_of("opponent_id"),
		)
	} else {
		let mut rng = thread_rng();

		match app.subcommand() {
			("local", Some(sub)) => run_vs_computer(
				&mut bot,
				Computer::new(
					sub.value_of("race").map_or(Race::Random, |race| {
						race.parse().expect("Can't parse computer race")
					}),
					sub.value_of("difficulty")
						.map_or(Difficulty::VeryEasy, |difficulty| {
							difficulty.parse().expect("Can't parse computer difficulty")
						}),
					sub.value_of("ai_build")
						.map(|ai_build| ai_build.parse().expect("Can't parse computer build")),
				),
				sub.value_of("map").unwrap_or_else(|| {
					[
						"AcropolisLE",
						"DiscoBloodbathLE",
						"EphemeronLE",
						"ThunderbirdLE",
						"TritonLE",
						"WintersGateLE",
						"WorldofSleepersLE",
					]
					.choose(&mut rng)
					.unwrap()
				}),
				LaunchOptions {
					sc2_version: sub.value_of("sc2_version"),
					realtime: sub.is_present("realtime"),
					save_replay_as: sub.value_of("save_replay"),
				},
			),
			("human", Some(sub)) => run_vs_human(
				&mut bot,
				PlayerSettings::new(
					sub.value_of("race")
						.unwrap()
						.parse()
						.expect("Can't parse human race"),
					sub.value_of("name"),
				),
				sub.value_of("map").unwrap_or_else(|| {
					[
						"AcropolisLE",
						"DiscoBloodbathLE",
						"EphemeronLE",
						"ThunderbirdLE",
						"TritonLE",
						"WintersGateLE",
						"WorldofSleepersLE",
					]
					.choose(&mut rng)
					.unwrap()
				}),
				LaunchOptions {
					sc2_version: sub.value_of("sc2_version"),
					realtime: true,
					save_replay_as: sub.value_of("save_replay"),
				},
			),
			_ => {
				println!("Game mode is not specified! Use -h, --help to print help information.");
				std::process::exit(0);
			}
		}
	}
}
