use super::tests::{alloc, device, read};
use super::*;

#[test]
fn optional_quad_provenance_survives_grow_move_trim_and_toggle() {
    let (device, queue) = device().expect("provenance regression needs a GPU adapter");
    let mut arena = Arena::new(&device, 24, GRAIN, true);
    let (page, start) = alloc(&mut arena, &device, &queue, GRAIN).unwrap();
    assert!(arena.provenance(page).is_none());
    assert!(arena.set_provenance(&device, true));
    let words = [0x1234_5678u32, 0x1543];
    queue.write_buffer(
        arena.provenance(page).unwrap(),
        start * 2,
        bytemuck::cast_slice(&words),
    );
    let old = arena.provenance(page).unwrap().clone();
    let (grown, _) = alloc(&mut arena, &device, &queue, GRAIN).unwrap();
    assert_eq!(page, grown);
    assert_ne!(&old, arena.provenance(page).unwrap());
    assert_eq!(
        read(&device, &queue, arena.provenance(page).unwrap(), start * 2),
        words[0]
    );
    assert_eq!(
        read(
            &device,
            &queue,
            arena.provenance(page).unwrap(),
            start * 2 + 4
        ),
        words[1]
    );
    assert_eq!(
        arena.provenance(page).unwrap().size() * 12,
        arena.buffer(page).size()
    );
    assert!(arena.open_page(&device, GRAIN));
    let moved = arena.pages.len() as u32 - 1;
    let mut encoder = device.create_command_encoder(&Default::default());
    arena.copy(&mut encoder, (page, start), (moved, 0), 4);
    queue.submit([encoder.finish()]);
    assert_eq!(
        read(&device, &queue, arena.provenance(moved).unwrap(), 0),
        words[0]
    );
    assert_eq!(
        read(&device, &queue, arena.provenance(moved).unwrap(), 4),
        words[1]
    );
    let moved_vertex = arena.buffer(moved).clone();
    let moved_meta = arena.provenance(moved).unwrap().clone();
    let removed = arena.trim();
    assert!(removed.contains(&moved_vertex));
    assert!(removed.contains(&moved_meta));
    assert!(arena.provenance(moved).is_none());
    assert!(arena.set_provenance(&device, false));
    assert!(arena.provenance(page).is_none());
}
