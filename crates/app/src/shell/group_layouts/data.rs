//! Intentionally varied sample namespaces, including parent and near-prefix names.
#[derive(Clone)]
pub(super) struct Group {
    pub name: String,
    pub articles: u64,
}
struct FakeGroup {
    name: &'static str,
    articles: u64,
}
const GROUPS: &[FakeGroup] = &[
    FakeGroup {
        name: "alt.binaries.pictures.nature",
        articles: 734_182,
    },
    FakeGroup {
        name: "comp.lang.rust",
        articles: 91_450,
    },
    FakeGroup {
        name: "alt.binaries",
        articles: 93_421,
    },
    FakeGroup {
        name: "alt.binaries.pictures.animals",
        articles: 24_060,
    },
    FakeGroup {
        name: "rec.photo.digital",
        articles: 8_462,
    },
    FakeGroup {
        name: "alt.binaries.pictures",
        articles: 202_111,
    },
    FakeGroup {
        name: "alt.binaries.pictures.architecture",
        articles: 1_623_970,
    },
    FakeGroup {
        name: "comp.lang.python",
        articles: 443_190,
    },
    FakeGroup {
        name: "alt.binaries.pictures2.nature",
        articles: 7_019,
    },
    FakeGroup {
        name: "alt.binaries.music.jazz",
        articles: 67_235,
    },
    FakeGroup {
        name: "alt.binaries.pictures.travel",
        articles: 305_040,
    },
    FakeGroup {
        name: "comp.lang",
        articles: 1_832,
    },
    FakeGroup {
        name: "rec.photo",
        articles: 136_729,
    },
    FakeGroup {
        name: "alt.binaries.pictures.space",
        articles: 3_918,
    },
    FakeGroup {
        name: "alt.binaries.music.classical",
        articles: 912_038,
    },
    FakeGroup {
        name: "comp.graphics.rendering",
        articles: 215_480,
    },
    FakeGroup {
        name: "alt.binaries.pictures.nature.birds",
        articles: 58_712,
    },
    FakeGroup {
        name: "alt.binaries.pictures.nature.landscapes",
        articles: 1_085_744,
    },
    FakeGroup {
        name: "comp.lang.rust.announce",
        articles: 428,
    },
    FakeGroup {
        name: "rec.photo.film",
        articles: 56_730,
    },
    FakeGroup {
        name: "alt.binaries2.pictures",
        articles: 413_672,
    },
    FakeGroup {
        name: "comp.graphics",
        articles: 19_325,
    },
    FakeGroup {
        name: "misc.test",
        articles: 109,
    },
    FakeGroup {
        name: "alt.test",
        articles: 28_641,
    },
];

pub(super) fn split(name: &str) -> (&str, &str) {
    name.rsplit_once('.').unwrap_or(("", name))
}
pub(super) fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(digit);
    }
    result
}

pub(super) fn fake() -> Vec<Group> {
    GROUPS
        .iter()
        .map(|group| Group {
            name: group.name.to_owned(),
            articles: group.articles,
        })
        .collect()
}
