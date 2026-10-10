use super::*;

#[test]
fn every_column_orders_filtered_indexes_both_ways_without_moving_groups() {
    let groups = vec![
        ListedGroup {
            name: "alt.z".into(),
            low: 10,
            high: 29,
            status: 'y',
        },
        ListedGroup {
            name: "alt.a".into(),
            low: 30,
            high: 34,
            status: 'n',
        },
        ListedGroup {
            name: "alt.m".into(),
            low: 20,
            high: 29,
            status: 'm',
        },
    ];
    let original = groups.clone();
    let subscribed = HashSet::from(["alt.z".to_string()]);
    for (column, ascending) in [
        (Column::Name, vec![1, 2, 0]),
        (Column::Subscribed, vec![1, 2, 0]),
        (Column::Articles, vec![1, 2, 0]),
        (Column::Posting, vec![1, 2, 0]),
    ] {
        let mut sort = PickerSort {
            column,
            descending: false,
        };
        let mut indexes = vec![0, 1, 2];
        sort.apply(&mut indexes, &groups, &subscribed);
        assert_eq!(indexes, ascending);
        sort.choose(column);
        sort.apply(&mut indexes, &groups, &subscribed);
        assert_eq!(indexes, ascending.into_iter().rev().collect::<Vec<_>>());
        assert_eq!(groups, original);
    }
    let mut indexes = vec![2, 0];
    PickerSort::default().apply(&mut indexes, &groups, &subscribed);
    assert_eq!(indexes, vec![2, 0]);
}
