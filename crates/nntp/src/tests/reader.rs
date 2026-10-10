//! Exact RFC3977 command/response transcripts, including stateful selection.
use super::*;
use crate::{ArticleRange, HeaderScope, NewsDateTime, RangeRef};

async fn transcript(cases: Vec<(&'static str, &'static [u8])>) -> Connection {
    let addr=serve(move |mut sock|{let cases=cases.clone();async move {
        greet(&mut sock).await;
        capabilities(&mut sock,"HDR\r\nNEWNEWS\r\nLIST ACTIVE ACTIVE.TIMES DISTRIB.PATS NEWSGROUPS HEADERS OVERVIEW.FMT\r\n").await;
        for (command,response) in cases {
            assert_eq!(read_line(&mut sock).await,command);
            write_all(&mut sock,response).await;
        }
    }}).await;
    Connection::connect(&plain_cfg(addr)).await.unwrap()
}

#[tokio::test]
async fn article_selection_current_forms_and_navigation_preserve_wire_semantics() {
    let mut conn = transcript(vec![
        (
            "LISTGROUP alt.test 5-",
            b"211 3 5 9 alt.test\r\n5\r\n7\r\n9\r\n.\r\n",
        ),
        ("LISTGROUP", b"211 3 5 9 alt.test\r\n5\r\n7\r\n9\r\n.\r\n"),
        ("STAT", b"223 5 <one@test> selected\r\n"),
        ("NEXT", b"223 7 <two@test>\r\n"),
        ("NEXT", b"223 9 <three@test>\r\n"),
        ("NEXT", b"422 no next article\r\n"),
        ("LAST", b"223 7 <two@test>\r\n"),
        (
            "HEAD",
            b"221 7 <two@test>\r\nSubject: reader test\r\nMessage-ID: <two@test>\r\n.\r\n",
        ),
        (
            "BODY",
            b"222 7 <two@test>\r\nfirst\r\n\r\n..literal\r\n.\r\n",
        ),
        (
            "ARTICLE",
            b"220 7 <two@test> article\r\nSubject: reader test\r\n\r\nfirst\r\n..literal\r\n.\r\n",
        ),
        (
            "ARTICLE <unselected@test>",
            b"220 0 <unselected@test>\r\nSubject: outside group\r\n\r\nbody\r\n.\r\n",
        ),
        ("STAT 5", b"223 5 <one@test>\r\n"),
        ("LAST", b"421 no previous article\r\n"),
        ("QUIT", b"205 closing\r\n"),
    ])
    .await;
    let group = conn
        .list_group(
            Some("alt.test"),
            Some(ArticleRange {
                first: 5,
                last: None,
            }),
        )
        .await
        .unwrap();
    assert_eq!(group.group.name, "alt.test");
    assert_eq!(group.articles, vec![5, 7, 9]);
    assert_eq!(
        conn.list_group(None, None).await.unwrap().articles,
        vec![5, 7, 9]
    );
    assert_eq!(
        conn.stat_identity(None).await.unwrap().message_id,
        "<one@test>"
    );
    assert_eq!(conn.next().await.unwrap().number, 7);
    assert_eq!(conn.next().await.unwrap().number, 9);
    assert!(matches!(conn.next().await, Err(Error::NoNextArticle)));
    assert_eq!(conn.last().await.unwrap().number, 7);
    assert!(conn
        .head_current()
        .await
        .unwrap()
        .starts_with(b"Subject: reader test\r\n"));
    assert_eq!(
        conn.body_current(100).await.unwrap(),
        b"first\r\n\r\n.literal\r\n"
    );
    let article = conn.article(None).await.unwrap();
    assert_eq!(article.identity.number, 7);
    assert_eq!(
        article.bytes,
        b"Subject: reader test\r\n\r\nfirst\r\n.literal\r\n"
    );
    let id = ArticleRef::MessageId("unselected@test".into());
    assert_eq!(conn.article(Some(&id)).await.unwrap().identity.number, 0);
    assert_eq!(conn.stat(&ArticleRef::Number(5)).await.unwrap(), 5);
    assert!(matches!(conn.last().await, Err(Error::NoPreviousArticle)));
    conn.quit().await.unwrap();
}

#[tokio::test]
async fn header_fields_overview_metadata_and_extensions_keep_empty_and_extra_fields() {
    let mut conn=transcript(vec![
        ("HDR Subject 5-9",b"225 Headers follow\r\n5 first subject\r\n7\r\n9 last subject\r\n.\r\n"),
        ("HDR :bytes <outside@test>",b"225 headers\r\n0 64\r\n.\r\n"),
        ("HDR Message-ID",b"225 headers\r\n7 <two@test>\r\n.\r\n"),
        ("LIST HEADERS",b"215 fields\r\n:\r\n:bytes\r\n:lines\r\n.\r\n"),
        ("LIST HEADERS MSGID",b"215 fields\r\nSubject\r\n.\r\n"),
        ("LIST HEADERS RANGE",b"215 fields\r\n:bytes\r\n.\r\n"),
        ("LIST OVERVIEW.FMT",b"215 fields\r\nSubject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\nXref:full\r\n.\r\n"),
        ("LIST OVERVIEW.FMT",b"215 fields\r\nSubject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\nXref:full\r\n.\r\n"),
        ("OVER 5-9",b"224 overview\r\n5\tfirst\tPoster\tDate\t<one@test>\t\t64\t2\tXref: server alt.test:5\r\n.\r\n"),
        ("OVER <outside@test>",b"224 overview\r\n0\tsubject\tPoster\tDate\t<outside@test>\t\t64\t2\tXref: server alt.other:1\r\n.\r\n"),
        ("OVER",b"224 overview\r\n7\tcurrent\tPoster\tDate\t<two@test>\t\t1\t1\r\n.\r\n"),
    ]).await;
    let range = RangeRef::Range(ArticleRange {
        first: 5,
        last: Some(9),
    });
    let headers = conn.hdr("Subject", &range).await.unwrap();
    assert_eq!(headers.len(), 3);
    assert_eq!(headers[1].article, 7);
    assert_eq!(headers[1].value, "");
    let outside = RangeRef::Article(ArticleRef::MessageId("outside@test".into()));
    assert_eq!(conn.hdr(":bytes", &outside).await.unwrap()[0].article, 0);
    assert_eq!(
        conn.hdr("Message-ID", &RangeRef::Current).await.unwrap()[0].value,
        "<two@test>"
    );
    assert_eq!(
        conn.list_headers(HeaderScope::All).await.unwrap(),
        [":", ":bytes", ":lines"]
    );
    assert_eq!(
        conn.list_headers(HeaderScope::MessageId).await.unwrap(),
        ["Subject"]
    );
    assert_eq!(
        conn.list_headers(HeaderScope::Range).await.unwrap(),
        [":bytes"]
    );
    assert_eq!(
        conn.list_overview_format().await.unwrap().last().unwrap(),
        "Xref:full"
    );
    assert_eq!(conn.over(&range).await.unwrap()[0].bytes, 64);
    let raw = conn.over_raw(&outside).await.unwrap();
    assert_eq!(raw[0][8], "Xref: server alt.other:1");
    assert_eq!(conn.over(&RangeRef::Current).await.unwrap()[0].article, 7);
}

#[tokio::test]
async fn group_discovery_help_lists_and_capability_refresh_are_complete_reader_queries() {
    let mut conn = transcript(vec![
        (
            "HELP",
            b"100 help follows\r\nReader help\r\n..literal\r\n.\r\n",
        ),
        (
            "NEWGROUPS 20261010 000060 GMT",
            b"231 new groups\r\nalt.new 9 5 y\r\n.\r\n",
        ),
        (
            "NEWNEWS alt.*,!alt.no 20261010 000060 GMT",
            b"230 new messages\r\n<one@test>\r\n<two@test>\r\n.\r\n",
        ),
        ("LIST", b"215 groups\r\nalt.test 9 5 y\r\n.\r\n"),
        (
            "LIST ACTIVE.TIMES alt.*",
            b"215 group creation\r\nalt.test 1234567890 admin@test\r\n.\r\n",
        ),
        (
            "LIST NEWSGROUPS alt.*",
            b"215 descriptions\r\nalt.test\tA test group with spaces\r\nalt.empty\r\n.\r\n",
        ),
        (
            "LIST DISTRIB.PATS",
            b"215 distributions\r\n10:alt.*:world\r\n20:alt.local.*:local\r\n.\r\n",
        ),
        (
            "LIST X-EXTENSION test",
            b"215 custom\r\nopaque extension value\r\n.\r\n",
        ),
        (
            "CAPABILITIES reader",
            b"101 changed caps\r\nVERSION 2\r\nREADER\r\nHDR\r\nLIST ACTIVE NEWSGROUPS\r\n.\r\n",
        ),
        ("MODE READER", b"200 reader posting allowed\r\n"),
        (
            "CAPABILITIES",
            b"101 after mode\r\nVERSION 2\r\nREADER\r\nXOVER\r\n.\r\n",
        ),
    ])
    .await;
    assert_eq!(conn.help().await.unwrap(), "Reader help\r\n.literal\r\n");
    let since = NewsDateTime {
        date: "20261010".into(),
        time: "000060".into(),
        gmt: true,
    };
    assert_eq!(conn.new_groups(&since).await.unwrap()[0].name, "alt.new");
    assert_eq!(
        conn.new_news("alt.*,!alt.no", &since).await.unwrap(),
        ["<one@test>", "<two@test>"]
    );
    assert_eq!(conn.list(None, None).await.unwrap()[0], b"alt.test 9 5 y");
    assert_eq!(
        conn.list_active_times(Some("alt.*")).await.unwrap()[0].created,
        1234567890
    );
    let groups = conn.list_newsgroups(Some("alt.*")).await.unwrap();
    assert_eq!(groups[0].description, "A test group with spaces");
    assert_eq!(groups[1].description, "");
    assert_eq!(
        conn.list_distribution_patterns().await.unwrap()[1].weight,
        20
    );
    assert_eq!(
        conn.list(Some("X-EXTENSION"), Some("test")).await.unwrap()[0],
        b"opaque extension value"
    );
    let caps = conn.request_capabilities(Some("reader")).await.unwrap();
    assert!(caps.supports("hdr"));
    assert!(caps.supports_list("newsgroups"));
    assert!(!caps.over);
    assert!(conn.mode_reader().await.unwrap());
    assert!(conn.capabilities().supports("XOVER"));
    assert!(!conn.capabilities().supports("HDR"));
}

#[tokio::test]
async fn reader_errors_invalid_arguments_and_bounded_article_are_explicit() {
    let mut conn = transcript(vec![
        ("NEXT", b"412 no group\r\n"),
        ("LAST", b"420 invalid current\r\n"),
        ("HDR Subject", b"502 permission denied\r\n"),
        ("STAT <gone@test>", b"430 missing\r\n"),
        ("STAT 1", b"223 malformed\r\n"),
        ("DATE", b"111 20261010000000\r\n"),
        (
            "ARTICLE 1",
            b"220 1 <one@test>\r\nthis exceeds the limit\r\n.\r\n",
        ),
    ])
    .await;
    assert!(matches!(conn.next().await, Err(Error::NoGroupSelected)));
    assert!(matches!(conn.last().await, Err(Error::NoCurrentArticle)));
    assert!(matches!(
        conn.hdr("Subject", &RangeRef::Current).await,
        Err(Error::PermissionDenied)
    ));
    assert!(matches!(
        conn.stat_identity(Some(&ArticleRef::MessageId("gone@test".into())))
            .await,
        Err(Error::Missing)
    ));
    assert!(matches!(
        conn.stat(&ArticleRef::Number(1)).await,
        Err(Error::Protocol { code: 0, .. })
    ));
    assert!(conn.group("alt.test other").await.is_err());
    assert!(conn
        .list_group(
            None,
            Some(ArticleRange {
                first: 1,
                last: None
            })
        )
        .await
        .is_err());
    assert!(conn
        .over(&RangeRef::Range(ArticleRange {
            first: 0,
            last: Some(10)
        }))
        .await
        .is_err());
    assert!(conn
        .hdr("Subject\r\nQUIT", &RangeRef::Current)
        .await
        .is_err());
    assert!(conn
        .article(Some(&ArticleRef::MessageId("a@test other".into())))
        .await
        .is_err());
    let bad = NewsDateTime {
        date: "20260230".into(),
        time: "000000".into(),
        gmt: true,
    };
    assert!(conn.new_groups(&bad).await.is_err());
    // Rejected arguments emitted no extra wire commands and left the session usable.
    assert_eq!(conn.date().await.unwrap(), "20261010000000");
    assert!(matches!(
        conn.article_limited(Some(&ArticleRef::Number(1)), 4).await,
        Err(Error::BodyTooLarge(4))
    ));
}

#[tokio::test]
async fn legacy_xhdr_capability_and_fallback_remain_usable() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "XHDR\r\n").await;
        assert_eq!(read_line(&mut sock).await, "XHDR Subject 5-");
        write_all(&mut sock, b"221 xhdr\r\n5 legacy subject\r\n.\r\n").await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let target = RangeRef::Range(ArticleRange {
        first: 5,
        last: None,
    });
    assert_eq!(
        conn.hdr("Subject", &target).await.unwrap()[0].value,
        "legacy subject"
    );
}

#[tokio::test]
async fn lossless_hdr_and_overview_keep_legacy_octets_empty_fields_and_extra_metadata() {
    let mut conn=transcript(vec![
        ("HDR Subject 5-9",b"225 Headers follow\r\n5 caf\xe9 \xff\r\n7\r\n9  leading space\r\n.\r\n"),
        ("OVER 5-9",b"224 Overview\r\n5\tcaf\xe9 \xff\tPoster\tDate\t<one@test>\t\t64\t2\tXref: server alt.test:5\t\xffextra\t\r\n.\r\n"),
        ("HDR Subject 5",b"225 headers\r\n5 caf\xe9\r\n.\r\n"),
    ]).await;
    let range = RangeRef::Range(ArticleRange {
        first: 5,
        last: Some(9),
    });
    let headers = conn.hdr_bytes("Subject", &range).await.unwrap();
    assert_eq!(headers[0].article, 5);
    assert_eq!(headers[0].value, b"caf\xe9 \xff");
    assert_eq!(headers[1].article, 7);
    assert!(headers[1].value.is_empty());
    assert_eq!(headers[2].value, b" leading space");
    let overview = conn.over_bytes(&range).await.unwrap();
    assert_eq!(
        overview[0],
        b"5\tcaf\xe9 \xff\tPoster\tDate\t<one@test>\t\t64\t2\tXref: server alt.test:5\t\xffextra\t"
    );
    // Existing display helper remains explicitly lossy and API-compatible.
    assert_eq!(
        conn.hdr("Subject", &RangeRef::Article(ArticleRef::Number(5)))
            .await
            .unwrap()[0]
            .value,
        "caf\u{fffd}"
    );
}
