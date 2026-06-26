//! Skill context budget benchmark — validates that reducing `context_budget_percent`
//! from 5% to 2% does not cause unacceptable skill loss across common context windows.
//!
//! Run with: `cargo bench -p xiaolin-core --bench skill_budget`
//!
//! Budget formula: `char_budget = context_window × percent / 100 × 4`
//!
//! Pass criteria:
//! - At 2% budget with 128K context, ≥80% of skills must be retained
//! - At 2% budget with 200K context, ≥90% of skills must be retained
//! - Compact mode must always outperform Full mode on retention

use std::path::PathBuf;

use xiaolin_core::config::SkillPromptMode;
use xiaolin_core::skill::{SkillEntry, SkillFrontmatter, SkillLayer, SkillRegistry};

fn make_realistic_skill(idx: usize) -> SkillEntry {
    let id = format!("skill-{idx:03}");
    let name = format!("Skill {idx}");
    let desc = format!(
        "This skill handles task category {cat} with specialization in area {area}. \
         It provides tools and guidance for common workflows.",
        cat = idx % 8,
        area = idx % 5,
    );
    let content = format!(
        "# {name}\n\n{desc}\n\n## Usage\n\nInvoke this skill when working on \
         category-{cat} tasks.\n\n## Details\n\nStep 1: Analyze the input.\n\
         Step 2: Apply the transformation.\nStep 3: Validate the output.\n",
        name = name,
        desc = desc,
        cat = idx % 8,
    );
    let when_to_use = if idx % 3 == 0 {
        Some(format!("Use when working on category-{} tasks", idx % 8))
    } else {
        None
    };

    let layer = match idx % 4 {
        0 => SkillLayer::Extension,
        1 => SkillLayer::Project,
        2 => SkillLayer::Global,
        _ => SkillLayer::AgentWorkspace,
    };

    SkillEntry {
        id: id.clone(),
        name,
        description: Some(desc),
        content,
        source_path: PathBuf::from(format!("/fake/{id}/SKILL.md")),
        frontmatter: SkillFrontmatter {
            name: Some(format!("Skill {idx}")),
            when_to_use,
            ..Default::default()
        },
        layer,
        source: None,
    }
}

fn build_registry(count: usize) -> SkillRegistry {
    let mut reg = SkillRegistry::new();
    for i in 0..count {
        reg.register(make_realistic_skill(i));
    }
    reg
}

fn calc_budget(context_window: u32, percent: u8) -> usize {
    (context_window as usize) * (percent as usize) / 100 * 4
}

struct BudgetResult {
    included: usize,
    retention_pct: f64,
}

fn run_budget_test(
    reg: &SkillRegistry,
    mode: &SkillPromptMode,
    mode_name: &'static str,
    context_window: u32,
    percent: u8,
) -> BudgetResult {
    let budget = calc_budget(context_window, percent);
    let (output, _info, ids) = reg.format_with_budget_ordered(mode, Some(budget), None);

    let total = reg
        .list()
        .iter()
        .filter(|s| s.frontmatter.enabled.unwrap_or(true))
        .count();
    let included = ids.len();
    let omitted = total.saturating_sub(included);
    let retention = if total > 0 {
        (included as f64 / total as f64) * 100.0
    } else {
        100.0
    };

    print!(
        "  {:>8} {:>5} {:>3}%  {:>6} {:>6} {:>6}  {:>6.1}%  {:>8} {:>8}\n",
        mode_name,
        format!("{}K", context_window / 1000),
        percent,
        total,
        included,
        omitted,
        retention,
        output.len(),
        budget,
    );

    BudgetResult {
        included,
        retention_pct: retention,
    }
}

fn main() {
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║          XiaoLin Skill Context Budget Benchmark                ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  Budget = context_window × percent / 100 × 4 (chars)           ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();

    let skill_counts = [50, 107, 150];
    let context_windows: &[(u32, &str)] = &[(32_000, " 32K"), (128_000, "128K"), (200_000, "200K")];
    let percents = [2u8, 5, 10];
    let modes: &[(&SkillPromptMode, &str)] = &[
        (&SkillPromptMode::Full, "Full"),
        (&SkillPromptMode::Compact, "Compact"),
        (&SkillPromptMode::Lazy, "Lazy"),
    ];

    let mut all_pass = true;

    for &count in &skill_counts {
        let reg = build_registry(count);
        println!("  ── {count} skills ──────────────────────────────────────────────");
        println!(
            "  {:>8} {:>5} {:>3}%  {:>6} {:>6} {:>6}  {:>7}  {:>8} {:>8}",
            "Mode", "CtxW", "Pct", "Total", "Incl", "Omit", "Retain", "OutChr", "Budget"
        );

        for &(mode, mode_name) in modes {
            for &(cw, _cw_label) in context_windows {
                for &pct in &percents {
                    let _r = run_budget_test(&reg, mode, mode_name, cw, pct);
                }
            }
        }
        println!();
    }

    // ── Pass/Fail assertions ──

    println!("  ── Assertions ──────────────────────────────────────────────────");

    // A1: 107 skills, Compact, 128K, 2% → ≥80% retention
    {
        let reg = build_registry(107);
        let r = run_budget_test(&reg, &SkillPromptMode::Compact, "Compact", 128_000, 2);
        let pass = r.retention_pct >= 80.0;
        println!(
            "  [{}] 107 skills, Compact, 128K, 2%: {:.1}% retention (≥80%)",
            if pass { "PASS" } else { "FAIL" },
            r.retention_pct
        );
        if !pass {
            all_pass = false;
        }
    }

    // A2: 107 skills, Compact, 200K, 2% → ≥90% retention
    {
        let reg = build_registry(107);
        let r = run_budget_test(&reg, &SkillPromptMode::Compact, "Compact", 200_000, 2);
        let pass = r.retention_pct >= 90.0;
        println!(
            "  [{}] 107 skills, Compact, 200K, 2%: {:.1}% retention (≥90%)",
            if pass { "PASS" } else { "FAIL" },
            r.retention_pct
        );
        if !pass {
            all_pass = false;
        }
    }

    // A3: Compact always retains ≥ Full mode (same budget)
    {
        let reg = build_registry(107);
        let compact = run_budget_test(&reg, &SkillPromptMode::Compact, "Compact", 128_000, 2);
        let full = run_budget_test(&reg, &SkillPromptMode::Full, "Full", 128_000, 2);
        let pass = compact.included >= full.included;
        println!(
            "  [{}] Compact ({}) ≥ Full ({}) retention at 128K/2%",
            if pass { "PASS" } else { "FAIL" },
            compact.included,
            full.included
        );
        if !pass {
            all_pass = false;
        }
    }

    // A4: Lazy mode retains 100% at 2%/128K (minimal format)
    {
        let reg = build_registry(107);
        let r = run_budget_test(&reg, &SkillPromptMode::Lazy, "Lazy", 128_000, 2);
        let pass = r.retention_pct >= 99.0;
        println!(
            "  [{}] 107 skills, Lazy, 128K, 2%: {:.1}% retention (≥99%)",
            if pass { "PASS" } else { "FAIL" },
            r.retention_pct
        );
        if !pass {
            all_pass = false;
        }
    }

    // A5: 150 skills, Compact, 128K, 2% → ≥55% (heavier load, edge case)
    {
        let reg = build_registry(150);
        let r = run_budget_test(&reg, &SkillPromptMode::Compact, "Compact", 128_000, 2);
        let pass = r.retention_pct >= 55.0;
        println!(
            "  [{}] 150 skills, Compact, 128K, 2%: {:.1}% retention (≥55%)",
            if pass { "PASS" } else { "FAIL" },
            r.retention_pct
        );
        if !pass {
            all_pass = false;
        }
    }

    // A6: 2% budget char size > 0 for all context windows
    {
        let pass_all = context_windows
            .iter()
            .all(|&(cw, _)| calc_budget(cw, 2) > 0);
        println!(
            "  [{}] 2% budget > 0 chars for all context windows",
            if pass_all { "PASS" } else { "FAIL" },
        );
        if !pass_all {
            all_pass = false;
        }
    }

    // A7: Baseline comparison — 2% vs 5% retention delta at 128K/Compact
    {
        let reg = build_registry(107);
        let r2 = run_budget_test(&reg, &SkillPromptMode::Compact, "Compact", 128_000, 2);
        let r5 = run_budget_test(&reg, &SkillPromptMode::Compact, "Compact", 128_000, 5);
        let delta = r5.retention_pct - r2.retention_pct;
        let pass = delta < 25.0;
        println!(
            "  [{}] 5% vs 2% retention delta at 128K: {:.1}pp (< 25pp)",
            if pass { "PASS" } else { "FAIL" },
            delta,
        );
        if !pass {
            all_pass = false;
        }
    }

    println!();
    if all_pass {
        println!("  ✓ All skill budget checks passed.");
    } else {
        println!("  ✗ Some skill budget checks FAILED!");
        std::process::exit(1);
    }
}
