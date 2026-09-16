use crate::camera::ThirdPersonCamera;
use crate::interaction::camera_interaction_ray;
use game_types::EntityId;
use sim_core::command::{Command, RobotCommandType};
use std::collections::BTreeSet;

/// 2D screen/NDC drag-selection marquee box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarqueeBox {
    /// Screen-space starting point in NDC [-1.0, 1.0].
    pub start_ndc: (f32, f32),
    /// Current screen-space drag point in NDC [-1.0, 1.0].
    pub current_ndc: (f32, f32),
}

impl MarqueeBox {
    pub fn new(start_ndc: (f32, f32)) -> Self {
        MarqueeBox {
            start_ndc,
            current_ndc: start_ndc,
        }
    }

    /// True if drag distance exceeds minimum threshold to differentiate from a single click.
    pub fn is_drag(&self) -> bool {
        let dx = self.current_ndc.0 - self.start_ndc.0;
        let dy = self.current_ndc.1 - self.start_ndc.1;
        dx * dx + dy * dy >= 0.001
    }

    pub fn min_x(&self) -> f32 {
        self.start_ndc.0.min(self.current_ndc.0)
    }

    pub fn max_x(&self) -> f32 {
        self.start_ndc.0.max(self.current_ndc.0)
    }

    pub fn min_y(&self) -> f32 {
        self.start_ndc.1.min(self.current_ndc.1)
    }

    pub fn max_y(&self) -> f32 {
        self.start_ndc.1.max(self.current_ndc.1)
    }
}

/// Tactical unit selection and squad commanding manager.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TacticalSelection {
    /// Entities currently selected by the player.
    pub selected_entities: BTreeSet<EntityId>,
    /// Primary selected entity (for HUD inspector details).
    pub primary_entity: Option<EntityId>,
    /// Active marquee drag-selection box, if mouse drag is in progress.
    pub marquee: Option<MarqueeBox>,
}

impl TacticalSelection {
    pub fn new() -> Self {
        TacticalSelection::default()
    }

    pub fn len(&self) -> usize {
        self.selected_entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected_entities.is_empty()
    }

    pub fn contains(&self, entity: EntityId) -> bool {
        self.selected_entities.contains(&entity)
    }

    pub fn clear(&mut self) {
        self.selected_entities.clear();
        self.primary_entity = None;
        self.marquee = None;
    }

    /// Select an individual entity. If `shift_append` is true, toggles/adds to existing selection.
    pub fn select_single(&mut self, entity: EntityId, shift_append: bool) {
        if !shift_append {
            self.selected_entities.clear();
            self.selected_entities.insert(entity);
            self.primary_entity = Some(entity);
        } else if self.selected_entities.contains(&entity) {
            self.selected_entities.remove(&entity);
            if self.primary_entity == Some(entity) {
                self.primary_entity = self.selected_entities.iter().next().copied();
            }
        } else {
            self.selected_entities.insert(entity);
            self.primary_entity = Some(entity);
        }
    }

    /// Casts an interaction ray through `(ndc_x, ndc_y)` and selects the closest candidate entity.
    /// Candidates are `(EntityId, world_pos, hit_radius)`.
    pub fn select_point(
        &mut self,
        camera: &ThirdPersonCamera,
        ndc_x: f32,
        ndc_y: f32,
        candidates: &[(EntityId, (f32, f32, f32), f32)],
        shift_append: bool,
    ) -> Option<EntityId> {
        let ray = camera_interaction_ray(camera, ndc_x, ndc_y);

        let mut closest_hit: Option<(EntityId, f32)> = None;

        for &(id, pos, radius) in candidates {
            // Test distance from candidate center to ray line
            let dx = pos.0 - ray.origin.0;
            let dy = pos.1 - ray.origin.1;
            let dz = pos.2 - ray.origin.2;

            // Dot product with ray direction
            let t = dx * ray.direction.0 + dy * ray.direction.1 + dz * ray.direction.2;
            if t <= 0.0 {
                continue;
            }

            // Closest point on ray to sphere center
            let px = ray.origin.0 + ray.direction.0 * t;
            let py = ray.origin.1 + ray.direction.1 * t;
            let pz = ray.origin.2 + ray.direction.2 * t;

            let dist_sq = (px - pos.0) * (px - pos.0)
                + (py - pos.1) * (py - pos.1)
                + (pz - pos.2) * (pz - pos.2);

            if dist_sq <= radius * radius {
                match closest_hit {
                    None => closest_hit = Some((id, t)),
                    Some((_, best_t)) if t < best_t => closest_hit = Some((id, t)),
                    _ => {}
                }
            }
        }

        if let Some((hit_id, _)) = closest_hit {
            self.select_single(hit_id, shift_append);
            Some(hit_id)
        } else {
            if !shift_append {
                self.clear();
            }
            None
        }
    }

    /// Start a marquee drag-selection box at `(ndc_x, ndc_y)`.
    pub fn start_marquee(&mut self, ndc_x: f32, ndc_y: f32) {
        self.marquee = Some(MarqueeBox::new((ndc_x, ndc_y)));
    }

    /// Update active marquee drag position.
    pub fn update_marquee(&mut self, ndc_x: f32, ndc_y: f32) {
        if let Some(ref mut m) = self.marquee {
            m.current_ndc = (ndc_x, ndc_y);
        }
    }

    /// Cancel active marquee drag.
    pub fn cancel_marquee(&mut self) {
        self.marquee = None;
    }

    /// Complete marquee selection, unprojecting corners onto ground plane ($Y=0$) and selecting
    /// all candidate entities falling within the ground footprint.
    /// Candidates are `(EntityId, world_pos)`.
    pub fn complete_marquee(
        &mut self,
        camera: &ThirdPersonCamera,
        candidates: &[(EntityId, (f32, f32, f32))],
        shift_append: bool,
    ) -> usize {
        let marquee = match self.marquee.take() {
            Some(m) if m.is_drag() => m,
            _ => return 0,
        };

        // Unproject 4 screen corners to ground plane (Y=0)
        let corners_ndc = [
            (marquee.min_x(), marquee.min_y()),
            (marquee.max_x(), marquee.min_y()),
            (marquee.min_x(), marquee.max_y()),
            (marquee.max_x(), marquee.max_y()),
        ];

        let mut ground_pts = Vec::new();
        for (nx, ny) in corners_ndc {
            let ray = camera_interaction_ray(camera, nx, ny);
            if let Some(pt) = ray.intersect_ground(0.0) {
                ground_pts.push(pt);
            }
        }

        if ground_pts.is_empty() {
            return 0;
        }

        let min_x = ground_pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let max_x = ground_pts
            .iter()
            .map(|p| p.0)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_z = ground_pts.iter().map(|p| p.2).fold(f32::INFINITY, f32::min);
        let max_z = ground_pts
            .iter()
            .map(|p| p.2)
            .fold(f32::NEG_INFINITY, f32::max);

        if !shift_append {
            self.selected_entities.clear();
            self.primary_entity = None;
        }

        let mut newly_selected = 0;
        for &(id, pos) in candidates {
            if pos.0 >= min_x && pos.0 <= max_x && pos.2 >= min_z && pos.2 <= max_z {
                if self.selected_entities.insert(id) {
                    newly_selected += 1;
                }
                if self.primary_entity.is_none() {
                    self.primary_entity = Some(id);
                }
            }
        }

        newly_selected
    }

    /// Generates authoritative server `Command::RobotCommand` move orders for all selected robots,
    /// distributed in formation offsets (2.5m spacing) around the focal `target_pos`.
    pub fn issue_move_order(&self, target_pos: (f32, f32, f32)) -> Vec<Command> {
        let count = self.selected_entities.len();
        if count == 0 {
            return Vec::new();
        }

        let mut commands = Vec::with_capacity(count);
        let cols = (count as f32).sqrt().ceil() as usize;
        let spacing = 2.5f32;

        for (i, &robot_id) in self.selected_entities.iter().enumerate() {
            let row = i / cols;
            let col = i % cols;

            let offset_x = (col as f32 - (cols as f32 - 1.0) * 0.5) * spacing;
            let offset_z = (row as f32) * spacing;

            let dest = (
                target_pos.0 + offset_x,
                target_pos.1,
                target_pos.2 + offset_z,
            );

            commands.push(Command::RobotCommand {
                robot_id,
                command_type: RobotCommandType::Move { position: dest },
            });
        }

        commands
    }

    /// Generates authoritative server `Command::RobotCommand` attack orders targeting an enemy entity.
    pub fn issue_attack_order(&self, target_entity: EntityId) -> Vec<Command> {
        self.selected_entities
            .iter()
            .map(|&robot_id| Command::RobotCommand {
                robot_id,
                command_type: RobotCommandType::Attack {
                    target: target_entity,
                },
            })
            .collect()
    }

    /// Generates authoritative server `Command::RobotCommand` guard orders for a ground location.
    pub fn issue_guard_order(&self, position: (f32, f32, f32)) -> Vec<Command> {
        self.selected_entities
            .iter()
            .map(|&robot_id| Command::RobotCommand {
                robot_id,
                command_type: RobotCommandType::Guard { position },
            })
            .collect()
    }

    /// Generates authoritative server `Command::RobotCommand` follow orders targeting a friendly entity.
    pub fn issue_follow_order(&self, target_entity: EntityId) -> Vec<Command> {
        self.selected_entities
            .iter()
            .map(|&robot_id| Command::RobotCommand {
                robot_id,
                command_type: RobotCommandType::Follow {
                    target: target_entity,
                },
            })
            .collect()
    }

    /// Generates authoritative server `Command::RobotCommand` orders commanding robots to return to base.
    pub fn issue_return_to_base_order(&self) -> Vec<Command> {
        self.selected_entities
            .iter()
            .map(|&robot_id| Command::RobotCommand {
                robot_id,
                command_type: RobotCommandType::ReturnToBase,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_and_marquee_selection() {
        let mut sel = TacticalSelection::new();
        let e1 = EntityId::new(10);
        let e2 = EntityId::new(20);
        let e3 = EntityId::new(30);

        // Single select
        sel.select_single(e1, false);
        assert_eq!(sel.len(), 1);
        assert!(sel.contains(e1));
        assert_eq!(sel.primary_entity, Some(e1));

        // Shift select appends e2
        sel.select_single(e2, true);
        assert_eq!(sel.len(), 2);
        assert!(sel.contains(e1));
        assert!(sel.contains(e2));

        // Shift select again on e2 removes e2
        sel.select_single(e2, true);
        assert_eq!(sel.len(), 1);
        assert!(!sel.contains(e2));

        // Replace select with e3
        sel.select_single(e3, false);
        assert_eq!(sel.len(), 1);
        assert!(sel.contains(e3));
        assert!(!sel.contains(e1));
    }

    #[test]
    fn test_tactical_order_generation() {
        let mut sel = TacticalSelection::new();
        let e1 = EntityId::new(101);
        let e2 = EntityId::new(102);
        sel.select_single(e1, true);
        sel.select_single(e2, true);

        // Issue move order
        let move_cmds = sel.issue_move_order((50.0, 0.0, 50.0));
        assert_eq!(move_cmds.len(), 2);
        for cmd in &move_cmds {
            match cmd {
                Command::RobotCommand {
                    robot_id,
                    command_type: RobotCommandType::Move { position },
                } => {
                    assert!(*robot_id == e1 || *robot_id == e2);
                    assert!((position.0 - 50.0).abs() <= 5.0);
                }
                _ => panic!("Expected RobotCommand::Move"),
            }
        }

        // Issue attack order
        let enemy = EntityId::new(999);
        let attack_cmds = sel.issue_attack_order(enemy);
        assert_eq!(attack_cmds.len(), 2);
        for cmd in &attack_cmds {
            match cmd {
                Command::RobotCommand {
                    command_type: RobotCommandType::Attack { target },
                    ..
                } => {
                    assert_eq!(*target, enemy);
                }
                _ => panic!("Expected RobotCommand::Attack"),
            }
        }
    }
}
