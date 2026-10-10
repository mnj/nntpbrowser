//! Discovery rows covering real dotted parents, similar prefixes, and a long tail.
use super::*;

pub(super) fn sample(done: bool) -> SampleSet {
    let mut groups: Vec<_> = [
        "alt",
        "alt.binaries",
        "alt.binaries.picture",
        "alt.binaries.picture.example",
        "alt.binaries.pictures",
        "alt.binaries.pictures.nature",
        "alt.binaries.pictures.nature.landscapes",
        "alt.binaries.pictures.nature.landscapes.alpine",
        "alt.binaries.pictures.photography",
        "alt.binaries.picturesque",
        "alt.binaries2",
        "comp",
        "comp.graphics",
        "comp.graphics.rendering",
        "rec.music.jazz",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        let low = 1 + index as u64 * 100;
        let count = 12_000 + ((index * 7) % 15) as u64 * 1100;
        ListedGroup {
            name: name.into(),
            low,
            high: low + count - 1,
            status: if name.starts_with("comp.") { 'm' } else { 'y' },
        }
    })
    .collect();
    groups.extend((0..2000 - groups.len() as u64).map(|index| ListedGroup {
        name: format!("test.samples.sample{index:04}"),
        low: 1,
        high: 40_000 + index,
        status: 'y',
    }));
    SampleSet {
        received: groups.len() as u64,
        articles: groups.iter().map(ListedGroup::posts).sum(),
        groups,
        done,
        started: Instant::now(),
    }
}
