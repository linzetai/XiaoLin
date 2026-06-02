use criterion::{black_box, criterion_group, criterion_main, Criterion};
use xiaolin_model_router::{BudgetTracker, ModelRouter, RoutingStrategy};

fn bench_route_matching(c: &mut Criterion) {
    let router_fixed = ModelRouter::new(RoutingStrategy::Fixed, BudgetTracker::new(None));
    let router_cost = ModelRouter::new(RoutingStrategy::CostOptimized, BudgetTracker::new(None));
    let router_fallback = ModelRouter::new(RoutingStrategy::Fallback, BudgetTracker::new(None));

    c.bench_function("route_fixed_small_context", |b| {
        b.iter(|| router_fixed.route(black_box(Some("gpt-4o")), black_box(500), None))
    });

    c.bench_function("route_fixed_large_context", |b| {
        b.iter(|| router_fixed.route(black_box(Some("gpt-4o")), black_box(100_000), None))
    });

    c.bench_function("route_cost_optimized_no_pref", |b| {
        b.iter(|| router_cost.route(black_box(None), black_box(4_000), None))
    });

    c.bench_function("route_fallback_small", |b| {
        b.iter(|| router_fallback.route(black_box(Some("gpt-4o")), black_box(1_000), None))
    });

    c.bench_function("route_cost_optimized_large", |b| {
        b.iter(|| router_cost.route(black_box(None), black_box(80_000), None))
    });
}

criterion_group!(benches, bench_route_matching);
criterion_main!(benches);
