use game_types::{FactionId, ResearchJobId, TechId};
use sim_core::modifier::{MODIFIER_SCALE, ModifierGroup, ModifierKind};
use sim_core::research::{ResearchJobState, ResearchManager};
use std::collections::BTreeMap;

/// Presentation state of a single node in the research tree.
///
/// Derived entirely from the authoritative [`ResearchManager`]; this module never
/// mutates simulation state and never decides an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TechNodeState {
    /// Completed and its unlocks/modifiers are live.
    Completed,
    /// Head of the queue and actively consuming ticks.
    InProgress,
    /// In the queue but not yet started.
    Queued,
    /// Prerequisites satisfied; can be queued now.
    Available,
    /// One or more prerequisites are still missing.
    Locked,
}

impl TechNodeState {
    pub const fn glyph(&self) -> char {
        match self {
            TechNodeState::Completed => '*',
            TechNodeState::InProgress => '>',
            TechNodeState::Queued => '+',
            TechNodeState::Available => 'o',
            TechNodeState::Locked => '.',
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            TechNodeState::Completed => "COMPLETE",
            TechNodeState::InProgress => "RESEARCHING",
            TechNodeState::Queued => "QUEUED",
            TechNodeState::Available => "AVAILABLE",
            TechNodeState::Locked => "LOCKED",
        }
    }
}

/// Visual representation of one technology node in the debug tech tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TechNodeVisual {
    pub tech_id: TechId,
    pub name: &'static str,
    pub tier: u8,
    pub state: TechNodeState,
    pub prerequisites: Vec<TechId>,
    pub unlock_count: usize,
    pub modifier_count: usize,
    pub duration_ticks: u32,
    /// RGBA color representation.
    pub color_rgba: (f32, f32, f32, f32),
}

/// Visual representation of one entry in the faction research queue.
#[derive(Debug, Clone, PartialEq)]
pub struct ResearchQueueEntryVisual {
    pub job_id: ResearchJobId,
    pub tech_id: TechId,
    pub name: &'static str,
    /// Zero-based position; 0 is the active job.
    pub position: usize,
    pub state: ResearchJobState,
    pub progress_ticks: u32,
    pub total_ticks: u32,
    pub progress_fraction: f32,
    pub color_rgba: (f32, f32, f32, f32),
}

/// Visual representation of one live faction-wide modifier.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveModifierVisual {
    pub kind: ModifierKind,
    pub display_name: &'static str,
    /// Combined multiplier in thousandths (`1000` == neutral).
    pub multiplier_milli: i64,
    /// Signed percentage delta from neutral, for direct display.
    pub percent_delta: f32,
    /// Per-stacking-group contribution breakdown, ascending by group.
    pub breakdown: Vec<(ModifierGroup, i64)>,
}

/// Aggregated telemetry summary of a faction's research progression.
#[derive(Debug, Clone, PartialEq)]
pub struct ResearchTelemetry {
    pub faction_id: FactionId,
    pub tech_tree_version: u32,
    pub total_techs: usize,
    pub completed_techs: usize,
    pub available_techs: usize,
    pub locked_techs: usize,
    pub queued_jobs: usize,
    pub active_tech: Option<TechId>,
    pub active_tech_name: Option<&'static str>,
    pub active_progress_fraction: f32,
    pub facilities_total: usize,
    pub facilities_ready: usize,
    pub modifier_patch_version: u64,
    pub active_modifier_count: usize,
}

/// Comprehensive client-side research visualization snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ResearchViewSnapshot {
    pub nodes: Vec<TechNodeVisual>,
    pub queue: Vec<ResearchQueueEntryVisual>,
    pub modifiers: Vec<ActiveModifierVisual>,
    pub telemetry: ResearchTelemetry,
}

impl ResearchViewSnapshot {
    /// Extracts a full visual and telemetry snapshot from authoritative research state.
    pub fn extract(research: &ResearchManager, target_faction: FactionId) -> Self {
        let queue_jobs = research.queue(target_faction);
        let queue_positions: BTreeMap<TechId, usize> = queue_jobs
            .iter()
            .enumerate()
            .map(|(idx, job)| (job.tech_id, idx))
            .collect();

        let mut nodes = Vec::new();
        let mut completed_count = 0usize;
        let mut available_count = 0usize;
        let mut locked_count = 0usize;

        for def in research.tech_tree.iter() {
            let state = if research.is_completed(target_faction, def.id) {
                completed_count += 1;
                TechNodeState::Completed
            } else if let Some(pos) = queue_positions.get(&def.id) {
                let job = &queue_jobs[*pos];
                if *pos == 0 && job.state == ResearchJobState::InProgress {
                    TechNodeState::InProgress
                } else {
                    TechNodeState::Queued
                }
            } else if def
                .prerequisites
                .iter()
                .all(|p| research.is_completed(target_faction, *p))
            {
                available_count += 1;
                TechNodeState::Available
            } else {
                locked_count += 1;
                TechNodeState::Locked
            };

            let color = match state {
                TechNodeState::Completed => (0.1, 0.95, 0.2, 1.0), // Vibrant green
                TechNodeState::InProgress => (0.2, 0.8, 1.0, 1.0), // Bright cyan
                TechNodeState::Queued => (1.0, 0.8, 0.1, 1.0),     // Warning amber
                TechNodeState::Available => (0.85, 0.85, 0.9, 1.0), // Bright neutral
                TechNodeState::Locked => (0.4, 0.4, 0.45, 0.6),    // Dim gray
            };

            nodes.push(TechNodeVisual {
                tech_id: def.id,
                name: def.name,
                tier: def.tier,
                state,
                prerequisites: def.prerequisites.to_vec(),
                unlock_count: def.unlocks.len(),
                modifier_count: def.modifiers.len(),
                duration_ticks: def.duration_ticks,
                color_rgba: color,
            });
        }

        let mut queue = Vec::with_capacity(queue_jobs.len());
        for (idx, job) in queue_jobs.iter().enumerate() {
            let name = research
                .tech_tree
                .get(job.tech_id)
                .map(|d| d.name)
                .unwrap_or("<unknown>");
            let color = match job.state {
                ResearchJobState::InProgress => (0.2, 0.8, 1.0, 1.0), // Bright cyan
                ResearchJobState::AwaitingResources => (1.0, 0.55, 0.1, 1.0), // Amber
                ResearchJobState::Unpowered => (0.9, 0.15, 0.15, 1.0), // Shutdown red
                ResearchJobState::Queued => (0.6, 0.6, 0.7, 0.8),     // Neutral gray
            };
            queue.push(ResearchQueueEntryVisual {
                job_id: job.id,
                tech_id: job.tech_id,
                name,
                position: idx,
                state: job.state,
                progress_ticks: job.progress_ticks,
                total_ticks: job.total_ticks,
                progress_fraction: job.progress_fraction(),
                color_rgba: color,
            });
        }

        let mut modifiers = Vec::new();
        for (kind, multiplier_milli) in research.modifiers.active_kinds(target_faction) {
            modifiers.push(ActiveModifierVisual {
                kind,
                display_name: kind.display_name(),
                multiplier_milli,
                percent_delta: (multiplier_milli - MODIFIER_SCALE) as f32 / 10.0,
                breakdown: research.modifiers.group_breakdown(target_faction, kind),
            });
        }

        let facilities_total = research
            .facilities
            .values()
            .filter(|f| f.faction_id == target_faction)
            .count();
        let facilities_ready = research
            .facilities
            .values()
            .filter(|f| f.faction_id == target_faction && f.can_research())
            .count();

        let active = queue.first();
        let telemetry = ResearchTelemetry {
            faction_id: target_faction,
            tech_tree_version: research.tech_tree.version(),
            total_techs: research.tech_tree.len(),
            completed_techs: completed_count,
            available_techs: available_count,
            locked_techs: locked_count,
            queued_jobs: queue.len(),
            active_tech: active.map(|e| e.tech_id),
            active_tech_name: active.map(|e| e.name),
            active_progress_fraction: active.map(|e| e.progress_fraction).unwrap_or(0.0),
            facilities_total,
            facilities_ready,
            modifier_patch_version: research.modifiers.version(),
            active_modifier_count: modifiers.len(),
        };

        ResearchViewSnapshot {
            nodes,
            queue,
            modifiers,
            telemetry,
        }
    }

    /// Nodes belonging to one presentation tier, in ascending tech id order.
    pub fn nodes_in_tier(&self, tier: u8) -> Vec<&TechNodeVisual> {
        self.nodes.iter().filter(|n| n.tier == tier).collect()
    }

    /// Highest tier present in the tree (0 when empty).
    pub fn max_tier(&self) -> u8 {
        self.nodes.iter().map(|n| n.tier).max().unwrap_or(0)
    }

    /// Generates a formatted ASCII diagnostic dashboard report.
    pub fn render_ascii_report(&self) -> String {
        let t = &self.telemetry;
        let mut out = String::new();
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str("|                    RESEARCH NETWORK STATUS                   |\n");
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str(&format!(
            "| Tech Tree v{:<3}  Completed: {:>3} / {:<3}   Available: {:>3}      |\n",
            t.tech_tree_version, t.completed_techs, t.total_techs, t.available_techs
        ));
        out.push_str(&format!(
            "| Facilities: {:>3} ready / {:<3} built      Queue Depth: {:>3}     |\n",
            t.facilities_ready, t.facilities_total, t.queued_jobs
        ));
        match (t.active_tech_name, t.active_tech) {
            (Some(name), Some(id)) => out.push_str(&format!(
                "| Active: {:<28} {:>6}  {:>5.1}%  |\n",
                name,
                format!("#{}", id.value()),
                t.active_progress_fraction * 100.0
            )),
            _ => out.push_str("| Active: <idle>                                               |\n"),
        }
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str(&format!(
            "| Modifier Patch v{:<6}      Active Modifiers: {:>3}            |\n",
            t.modifier_patch_version, t.active_modifier_count
        ));
        for m in &self.modifiers {
            out.push_str(&format!(
                "|   {:<26} x{:<6.3} ({:>+6.1}%)          |\n",
                m.display_name,
                m.multiplier_milli as f32 / MODIFIER_SCALE as f32,
                m.percent_delta
            ));
        }
        out.push_str("+--------------------------------------------------------------+\n");
        for entry in &self.queue {
            out.push_str(&format!(
                "| [{:>2}] {:<26} {:<12} {:>4}/{:<4}   |\n",
                entry.position,
                entry.name,
                match entry.state {
                    ResearchJobState::InProgress => "IN PROGRESS",
                    ResearchJobState::AwaitingResources => "NO MATERIALS",
                    ResearchJobState::Unpowered => "UNPOWERED",
                    ResearchJobState::Queued => "QUEUED",
                },
                entry.progress_ticks,
                entry.total_ticks
            ));
        }
        out.push_str("+--------------------------------------------------------------+\n");
        out
    }

    /// Generates an ASCII tech tree laid out by tier with prerequisite edges.
    pub fn render_ascii_tree(&self) -> String {
        let mut out = String::new();
        out.push_str("TECH TREE  (* complete  > researching  + queued  o available  . locked)\n");
        for tier in 1..=self.max_tier() {
            let nodes = self.nodes_in_tier(tier);
            if nodes.is_empty() {
                continue;
            }
            out.push_str(&format!("-- Tier {tier} "));
            out.push_str(&"-".repeat(52));
            out.push('\n');
            for node in nodes {
                let prereqs = if node.prerequisites.is_empty() {
                    "-".to_string()
                } else {
                    node.prerequisites
                        .iter()
                        .map(|p| format!("#{}", p.value()))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                out.push_str(&format!(
                    "  {} #{:<5} {:<30} {:<12} req:{:<10} +{}u/{}m\n",
                    node.state.glyph(),
                    node.tech_id.value(),
                    node.name,
                    node.state.label(),
                    prereqs,
                    node.unlock_count,
                    node.modifier_count
                ));
            }
        }
        out
    }
}
