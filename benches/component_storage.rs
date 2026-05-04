use std::collections::HashMap;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use dumpster_fire_engine::resource_manager::component::*;

const N: usize = 10_000;

struct EntityMap {
    components: HashMap<ComponentType, Component>,
}

impl EntityMap {
    fn new() -> Self { Self { components: HashMap::new() } }
    fn add(&mut self, c: Component) { self.components.insert(c.component_type(), c); }
    fn get(&self, t: ComponentType) -> Option<&Component> { self.components.get(&t) }
    fn has(&self, t: ComponentType) -> bool { self.components.contains_key(&t) }
}

struct EntityArr {
    components: [Option<Component>; ComponentType::COUNT],
}

impl EntityArr {
    fn new() -> Self { Self { components: [const { None }; ComponentType::COUNT] } }
    fn add(&mut self, c: Component) {
        let idx = c.component_type().index();
        self.components[idx] = Some(c);
    }
    fn get(&self, t: ComponentType) -> Option<&Component> { self.components[t.index()].as_ref() }
    fn has(&self, t: ComponentType) -> bool { self.components[t.index()].is_some() }
}

fn make_physics() -> Component {
    Component::Physics(PhysicsComponent {
        mass: 80.0,
        velocity: (0.0, 0.0, 0.0),
        acceleration: (0.0, -9.8, 0.0),
    })
}

fn make_transform() -> Component {
    Component::Transform(TransformComponent {
        position: (0.0, 0.0, 0.0),
        rotation: (0.0, 0.0, 0.0),
        scale: (1.0, 1.0, 1.0),
        _transform: true,
    })
}

fn build_map_entities(n: usize) -> Vec<EntityMap> {
    (0..n)
        .map(|i| {
            let mut e = EntityMap::new();
            e.add(make_physics());
            if i % 2 == 0 { e.add(make_transform()); }
            e
        })
        .collect()
}

fn build_arr_entities(n: usize) -> Vec<EntityArr> {
    (0..n)
        .map(|i| {
            let mut e = EntityArr::new();
            e.add(make_physics());
            if i % 2 == 0 { e.add(make_transform()); }
            e
        })
        .collect()
}

fn bench_lookup(c: &mut Criterion) {
    let mut map_entity = EntityMap::new();
    map_entity.add(make_physics());
    map_entity.add(make_transform());

    let mut arr_entity = EntityArr::new();
    arr_entity.add(make_physics());
    arr_entity.add(make_transform());

    let mut g = c.benchmark_group("lookup_single_entity");
    g.bench_function("hashmap/get", |b| {
        b.iter(|| black_box(map_entity.get(black_box(ComponentType::Physics))))
    });
    g.bench_function("array/get", |b| {
        b.iter(|| black_box(arr_entity.get(black_box(ComponentType::Physics))))
    });
    g.bench_function("hashmap/has", |b| {
        b.iter(|| black_box(map_entity.has(black_box(ComponentType::Physics))))
    });
    g.bench_function("array/has", |b| {
        b.iter(|| black_box(arr_entity.has(black_box(ComponentType::Physics))))
    });
    g.finish();
}

fn bench_scan(c: &mut Criterion) {
    let map_entities = build_map_entities(N);
    let arr_entities = build_arr_entities(N);

    let mut g = c.benchmark_group("scan_10k_for_physics");
    g.bench_function("hashmap", |b| {
        b.iter(|| {
            let mut count = 0usize;
            for e in &map_entities {
                if e.has(ComponentType::Physics) { count += 1; }
            }
            black_box(count)
        })
    });
    g.bench_function("array", |b| {
        b.iter(|| {
            let mut count = 0usize;
            for e in &arr_entities {
                if e.has(ComponentType::Physics) { count += 1; }
            }
            black_box(count)
        })
    });
    g.finish();
}

criterion_group!(benches, bench_lookup, bench_scan);
criterion_main!(benches);
