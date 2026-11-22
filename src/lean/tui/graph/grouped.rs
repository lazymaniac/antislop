//! Flat work cards. The existing layout and viewport-bounded canvas own geometry.
use super::*;
use crate::lean::tui::work_units::WorkUnit;

pub(super) fn draw(
    view: &View,
    canvas: &mut Canvas,
    node: &Node,
    unit: &WorkUnit,
    selected: bool,
    _critical: bool,
    theme: &Theme,
) {
    let fill = if selected {
        theme.selected()
    } else {
        theme.card()
    };
    // Clip before painting: even an enormous graph only touches viewport cells.
    let left = node.x.max(canvas.origin_x);
    let right = node
        .right()
        .min(canvas.origin_x.saturating_add(canvas.width));
    let top = node.y.max(canvas.origin_y);
    let bottom = node
        .y
        .saturating_add(node.height)
        .min(canvas.origin_y.saturating_add(canvas.height));
    if left >= right || top >= bottom {
        return;
    }
    for y in top..bottom {
        for x in left..right {
            canvas.put(x, y, ' ', fill);
        }
    }
    let width = usize::from(node.width.saturating_sub(4));
    let health = unit.health(view);
    let stage = unit.active_stage(view);
    let kind = unit
        .task
        .as_deref()
        .and_then(|id| view.tasks.iter().find(|task| task.id == id))
        .map(|task| task.kind.as_str());
    let label = if health == Health::Working {
        kind.filter(|kind| !kind.is_empty()).unwrap_or("working")
    } else {
        health.name()
    };
    let state = format!("{} {}", theme.status_glyph(health.name()), label);
    let elapsed = super::super::flow_editor::format_clock_mm_ss(unit.elapsed_ms(view));
    let state_width = width.saturating_sub(elapsed.width() + 1);
    canvas.text(
        node.x + 2,
        node.y + 1,
        &theme.fit(&state, state_width),
        fill.patch(theme.status(health.name())),
    );
    if elapsed.width() < width {
        canvas.text(
            node.right().saturating_sub(elapsed.width() as u16 + 2),
            node.y + 1,
            &elapsed,
            fill.patch(theme.dim()),
        );
    }
    if node.height > 3 {
        canvas.text(
            node.x + 2,
            node.y + if node.height > 4 { 3 } else { 2 },
            &theme.fit(&clean(&unit.title), width),
            fill.patch(theme.text()),
        );
    }
    if node.collapsed || node.height <= 4 {
        return;
    }
    let failure = unit
        .stages
        .iter()
        .filter_map(|stage| stage.id.0.split_once(':').map(|(_, id)| id))
        .filter_map(|id| view.tasks.iter().find(|task| task.id == id))
        .find_map(|task| task.reason.as_deref());
    let activity = failure
        .or_else(|| {
            view.agents
                .get(&stage.id.0)
                .map(|agent| agent.activity.as_str())
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    (unit.card == ORCHESTRATOR)
                        .then_some(view.activity.as_deref())
                        .flatten()
                })
                .or_else(|| {
                    unit.task
                        .as_deref()
                        .and_then(|id| view.tasks.iter().find(|task| task.id == id))
                        .map(|task| task.activity.as_str())
                        .filter(|text| !text.is_empty())
                })
        })
        .unwrap_or(if health == Health::Problem {
            "cause not reported"
        } else {
            "Waiting for work"
        });
    canvas.text(
        node.x + 2,
        node.y + 4,
        &theme.fit(&clean(activity), width),
        fill.patch(theme.dim()),
    );
    let mut x = node.x + 2;
    for stage in &unit.stages {
        let health = health::health(view, &stage.id.0);
        let label = format!(
            "{} {}  ",
            if health == Health::Working {
                if theme.ascii { ">" } else { "›" }
            } else {
                theme.status_glyph(health.name())
            },
            stage.label.to_lowercase()
        );
        let remaining = node.right().saturating_sub(x + 2);
        if remaining == 0 {
            break;
        }
        let shown = theme.fit(&label, usize::from(remaining));
        let style = if matches!(health, Health::Done | Health::Working) {
            theme.accent()
        } else {
            theme.dim()
        };
        canvas.text(x, node.y + 6, &shown, fill.patch(style));
        x += shown.width() as u16;
    }
    let decisions = unit.decisions(view);
    if decisions > 0 {
        canvas.text(
            node.x + 2,
            node.y + 5,
            &theme.fit(
                &theme.chrome(&format!("{decisions} pending decision(s) · d open")),
                width,
            ),
            fill.patch(theme.decision()),
        );
    }
}
