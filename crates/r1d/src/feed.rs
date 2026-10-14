//! Feeds for `http.rss` (docs/specs/apps.md §6): RSS 2.0, RSS 1.0 (RDF) and Atom, read into one shape. Text is
//! plain: markup in titles and summaries is stripped and entities decoded, so an app shows it as text (it never
//! renders a feed's HTML). Dates become UTC ISO strings, like the app's own datetimes.
//!
//! Ported from eneeware apps/desktop/src-tauri/src/feed.rs at 61bc7a7.

use chrono::DateTime;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde::Serialize;

/// Items read from a feed, at most (newest first, as feeds list them).
pub const MAX_ITEMS: usize = 100;
/// A summary is cut to about this many characters, at a word.
const SUMMARY_CHARS: usize = 280;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Feed {
    pub title: String,
    pub link: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Item {
    /// The item's id: its `guid` (RSS) or `id` (Atom), else its link, else its title and date.
    pub guid: String,
    pub title: String,
    pub link: Option<String>,
    /// Plain text, cut to about [`SUMMARY_CHARS`].
    pub summary: String,
    pub author: Option<String>,
    /// UTC ISO (`2026-10-06T12:00:00.000Z`), when the feed gives a date that parses.
    pub published: Option<String>,
    /// Its picture, an https URL: `media:thumbnail`, `media:content`, an image `enclosure`, `itunes:image`, or the
    /// first `<img>` in its text (a chat's headlines card shows it, crate::cards).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// The element names this reads, prefixed ones included (`dc:creator`). Any other prefixed element is skipped, so a
/// `media:title` doesn't stand in for the item's title.
fn known(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "rss" | "channel" | "feed" | "rdf:rdf" => "container",
        "item" | "entry" => "item",
        "title" => "title",
        "link" => "link",
        "guid" | "id" => "id",
        "description" | "summary" => "summary",
        "content:encoded" | "content" => "content",
        "author" | "dc:creator" => "author",
        "name" => "name",
        "pubdate" | "published" | "dc:date" => "published",
        "updated" => "updated",
        _ => return None,
    })
}

/// An element being read: its kind, the text inside it so far (nested elements' text included), and the item it's in.
struct Open {
    kind: Option<&'static str>,
    text: String,
}

#[derive(Default)]
struct Draft {
    id: Option<String>,
    title: Option<String>,
    link: Option<String>,
    summary: Option<String>,
    content: Option<String>,
    author: Option<String>,
    published: Option<String>,
    updated: Option<String>,
    image: Option<String>,
}

/// Reads a feed. Errors say why it isn't one.
pub fn parse(xml: &str) -> Result<Feed, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Open> = Vec::new();
    let mut feed = Feed::default();
    let mut item: Option<Draft> = None;
    let mut seen_container = false;
    loop {
        let event = reader.read_event().map_err(|e| format!("not a feed: {e}"))?;
        match event {
            Event::Start(e) => {
                if let Some(draft) = item.as_mut() {
                    image_attribute(&e, draft);
                }
                let kind = kind_of(&e);
                seen_container |= kind == Some("container");
                if kind == Some("item") && item.is_none() && feed.items.len() < MAX_ITEMS {
                    item = Some(Draft::default());
                }
                // Atom links carry their URL as an attribute.
                if kind == Some("link") {
                    link_attribute(&e, item.as_mut(), &mut feed, &stack);
                }
                stack.push(Open { kind, text: String::new() });
            }
            Event::Empty(e) if kind_of(&e) == Some("link") => link_attribute(&e, item.as_mut(), &mut feed, &stack),
            Event::Empty(e) => {
                if let Some(draft) = item.as_mut() {
                    image_attribute(&e, draft);
                }
            }
            Event::Text(t) => push_text(&mut stack, &t.html_content()),
            Event::CData(t) => push_text(&mut stack, &t.html_content()),
            Event::GeneralRef(r) => {
                let text = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match &*r.into_inner() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        // An HTML entity in a feed (`&nbsp;`): decoded with the summary's other entities.
                        other => format!("&{other};"),
                    },
                };
                push_text(&mut stack, &text);
            }
            Event::End(_) => {
                let Some(open) = stack.pop() else { continue };
                // The text belongs to the element that ends, and to the ones around it (Atom's xhtml content).
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&open.text);
                }
                let in_item = stack.iter().any(|o| o.kind == Some("item"));
                match (open.kind, item.as_mut()) {
                    (Some("item"), Some(_)) if !in_item => {
                        if let Some(done) = item.take().and_then(finish) {
                            feed.items.push(done);
                        }
                    }
                    (Some(kind), Some(draft)) if in_item => set(draft, kind, open.text, &stack),
                    (Some("title"), None) if feed.title.is_empty() && depth_in_container(&stack) => {
                        feed.title = clean(&open.text);
                    }
                    (Some("link"), None) if feed.link.is_none() && depth_in_container(&stack) => {
                        feed.link = url(&open.text);
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !seen_container {
        return Err("not an RSS or Atom feed".into());
    }
    Ok(feed)
}

fn kind_of(e: &BytesStart<'_>) -> Option<&'static str> {
    known(e.name().as_ref())
}

fn push_text(stack: &mut [Open], text: &str) {
    if let Some(open) = stack.last_mut() {
        open.text.push_str(text);
    }
}

/// The element just closed sits right inside the feed or channel (not in an item, an image or a source).
fn depth_in_container(stack: &[Open]) -> bool {
    stack.last().is_some_and(|o| o.kind == Some("container"))
}

/// An Atom `<link href>`: the alternate one (or one without `rel`) is the page.
fn link_attribute(e: &BytesStart<'_>, item: Option<&mut Draft>, feed: &mut Feed, stack: &[Open]) {
    let attr = |name: &str| {
        let found = e.try_get_attribute(name).ok().flatten()?;
        found.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.into_owned())
    };
    let Some(href) = attr("href") else { return };
    if attr("rel").is_some_and(|rel| rel != "alternate") {
        return;
    }
    match item {
        Some(draft) if draft.link.is_none() => draft.link = url(&href),
        None if feed.link.is_none() && depth_in_container(stack) => feed.link = url(&href),
        _ => {}
    }
}

/// An item's picture from its media elements (the first one found).
fn image_attribute(e: &BytesStart<'_>, draft: &mut Draft) {
    if draft.image.is_some() {
        return;
    }
    let attr = |name: &str| {
        let found = e.try_get_attribute(name).ok().flatten()?;
        found.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.into_owned())
    };
    let qname = e.name();
    let raw: &str = qname.as_ref();
    let name = raw.to_ascii_lowercase();
    let src = match name.as_str() {
        "media:thumbnail" => attr("url"),
        "itunes:image" => attr("href"),
        "media:content" | "enclosure" => {
            let typed = attr("type").is_some_and(|t| t.starts_with("image/")) || attr("medium").is_some_and(|m| m == "image");
            let looks = attr("url").is_some_and(|u| looks_like_image(&u));
            if typed || (looks && attr("type").is_none()) { attr("url") } else { None }
        }
        _ => None,
    };
    draft.image = src.as_deref().and_then(picture);
}

fn looks_like_image(u: &str) -> bool {
    let path = u.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".gif"].iter().any(|x| path.ends_with(x))
}

/// A picture's URL: https only (r1d fetches it for a card; crate::cards).
fn picture(text: &str) -> Option<String> {
    let text = decode_entities(text.trim());
    text.starts_with("https://").then_some(text)
}

/// The first `<img src>` in an item's HTML.
fn first_img(html: &str) -> Option<String> {
    let at = html.find("<img")?;
    let tag = &html[at..html[at..].find('>').map_or(html.len(), |e| at + e)];
    let src = tag.find("src=")? + 4;
    let rest = &tag[src..];
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let end = rest[1..].find(quote)?;
    picture(&rest[1..1 + end])
}

/// Keeps an item's field (the first of each; an Atom author's `name` is its author).
fn set(draft: &mut Draft, kind: &str, text: String, stack: &[Open]) {
    // Only the item's own fields: not a `title` inside its `source`, or a `name` outside its `author`.
    let parent = stack.last().and_then(|o| o.kind);
    let slot = match (kind, parent) {
        ("name", Some("author")) => &mut draft.author,
        ("name", _) => return,
        (_, Some("item")) => match kind {
            "id" => &mut draft.id,
            "title" => &mut draft.title,
            "link" => &mut draft.link,
            "summary" => &mut draft.summary,
            "content" => &mut draft.content,
            "author" => &mut draft.author,
            "published" => &mut draft.published,
            "updated" => &mut draft.updated,
            _ => return,
        },
        _ => return,
    };
    if slot.is_none() && !text.trim().is_empty() {
        *slot = Some(text);
    }
}

fn finish(d: Draft) -> Option<Item> {
    let title = d.title.as_deref().map(clean).unwrap_or_default();
    let link = d.link.as_deref().and_then(url);
    let summary = summarize(d.summary.as_deref().or(d.content.as_deref()).unwrap_or_default());
    let published = d.published.as_deref().or(d.updated.as_deref()).and_then(iso_date);
    let guid =
        d.id.map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .or_else(|| link.clone())
            .unwrap_or_else(|| format!("{title}|{}", published.as_deref().unwrap_or_default()));
    if title.is_empty() && summary.is_empty() && link.is_none() {
        return None;
    }
    // RSS authors are often `email (Name)`: the name is what to show.
    let author = d.author.as_deref().map(clean).map(|a| match (a.find('('), a.rfind(')')) {
        (Some(open), Some(close)) if open < close && a.contains('@') => a[open + 1..close].trim().to_string(),
        _ => a,
    });
    let image = d.image.or_else(|| first_img(d.content.as_deref().unwrap_or_default())).or_else(|| first_img(d.summary.as_deref().unwrap_or_default()));
    Some(Item { guid, title, link, summary, author: author.filter(|a| !a.is_empty()), published, image })
}

/// An http(s) URL, trimmed; anything else (a `javascript:` link) is dropped.
fn url(text: &str) -> Option<String> {
    let text = text.trim();
    (text.starts_with("https://") || text.starts_with("http://")).then(|| text.to_string())
}

/// A title or a name: HTML entities decoded and whitespace collapsed. Angle brackets stay: a title's `<` is text.
fn clean(text: &str) -> String {
    decode_entities(text).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text without markup: tags removed, HTML entities decoded, whitespace collapsed.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The HTML entities feeds use (a feed's own XML entities were decoded when it was read).
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail[..tail.len().min(12)].find(';') else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A summary: plain text, cut at a word near [`SUMMARY_CHARS`].
fn summarize(html: &str) -> String {
    let text = plain(html);
    if text.chars().count() <= SUMMARY_CHARS {
        return text;
    }
    let cut: String = text.chars().take(SUMMARY_CHARS).collect();
    let at = cut.rfind(' ').filter(|&i| i > SUMMARY_CHARS / 2).unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end_matches([',', ';', ':', '.', ' ']))
}

/// RFC 2822 (RSS), RFC 3339 (Atom) or a bare date (`dc:date`), as UTC ISO.
fn iso_date(text: &str) -> Option<String> {
    let text = text.trim();
    let ms = DateTime::parse_from_rfc2822(text)
        .or_else(|_| DateTime::parse_from_rfc3339(text))
        .or_else(|_| DateTime::parse_from_rfc3339(&format!("{text}T00:00:00Z")))
        .ok()?
        .timestamp_millis();
    Some(enee_spec::eval::iso_utc(ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures() {
        let xml = r#"<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel><title>N</title>
            <item><title>Thumb</title><media:thumbnail url="https://img.example/a.jpg" width="140"/></item>
            <item><title>Content</title><media:content url="https://img.example/v.mp4" type="video/mp4"/>
              <media:content url="https://img.example/b.png?w=600" medium="image"/></item>
            <item><title>Enclosure</title><enclosure url="https://img.example/c.webp" type="image/webp" length="1"/></item>
            <item><title>Html</title><description><![CDATA[<p><img alt="x" src="https://img.example/d.gif?a=1&amp;b=2"> words</p>]]></description></item>
            <item><title>Insecure</title><media:thumbnail url="http://img.example/e.jpg"/></item>
          </channel></rss>"#;
        let f = parse(xml).unwrap();
        let pics: Vec<Option<&str>> = f.items.iter().map(|i| i.image.as_deref()).collect();
        assert_eq!(
            pics,
            [
                Some("https://img.example/a.jpg"),
                Some("https://img.example/b.png?w=600"),
                Some("https://img.example/c.webp"),
                Some("https://img.example/d.gif?a=1&b=2"),
                None,
            ]
        );
    }

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:media="http://search.yahoo.com/mrss/">
  <channel>
    <title>Hacker News: Front Page</title>
    <link>https://news.ycombinator.com/</link>
    <image><title>Logo</title><url>https://example.com/logo.png</url></image>
    <item>
      <title>Show HN: A &amp; B &lt;fast&gt;</title>
      <description><![CDATA[<p>Article URL: <a href="https://example.com">https://example.com</a></p><p>Points:&nbsp;42</p>]]></description>
      <pubDate>Tue, 06 Oct 2026 12:30:00 +0000</pubDate>
      <link>https://example.com/a</link>
      <dc:creator>pg</dc:creator>
      <guid isPermaLink="false">https://news.ycombinator.com/item?id=1</guid>
      <media:title>Not the title</media:title>
    </item>
    <item>
      <title>Second</title>
      <link>https://example.com/b</link>
      <author>someone@example.com (Some One)</author>
      <pubDate>Mon, 05 Oct 2026 08:00:00 GMT</pubDate>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn rss_items_read_as_plain_text_with_utc_dates() {
        let feed = parse(RSS).unwrap();
        assert_eq!(feed.title, "Hacker News: Front Page", "not the image's title");
        assert_eq!(feed.link.as_deref(), Some("https://news.ycombinator.com/"));
        assert_eq!(feed.items.len(), 2);
        let a = &feed.items[0];
        assert_eq!(a.title, "Show HN: A & B <fast>", "XML entities decoded, and not treated as markup");
        assert_eq!(a.guid, "https://news.ycombinator.com/item?id=1");
        assert_eq!(a.link.as_deref(), Some("https://example.com/a"));
        assert_eq!(a.summary, "Article URL: https://example.com Points: 42");
        assert_eq!(a.author.as_deref(), Some("pg"));
        assert_eq!(a.published.as_deref(), Some("2026-10-06T12:30:00.000Z"));
        let b = &feed.items[1];
        assert_eq!(b.guid, "https://example.com/b", "no guid: the link");
        assert_eq!(b.author.as_deref(), Some("Some One"));
        assert_eq!(b.published.as_deref(), Some("2026-10-05T08:00:00.000Z"));
    }

    #[test]
    fn atom_entries_read_the_same_way() {
        let atom = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title type="text">Example blog</title>
  <link rel="self" href="https://example.com/feed.xml"/>
  <link href="https://example.com/"/>
  <entry>
    <title>Post one</title>
    <link rel="alternate" href="https://example.com/1"/>
    <link rel="enclosure" href="https://example.com/1.mp3"/>
    <id>tag:example.com,2026:1</id>
    <updated>2026-10-06T09:15:00+02:00</updated>
    <author><name>Ada</name></author>
    <content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml"><p>Hello <b>world</b></p></div></content>
  </entry>
</feed>"#;
        let feed = parse(atom).unwrap();
        assert_eq!(feed.title, "Example blog");
        assert_eq!(feed.link.as_deref(), Some("https://example.com/"), "the alternate link, not self");
        let e = &feed.items[0];
        assert_eq!((e.guid.as_str(), e.title.as_str()), ("tag:example.com,2026:1", "Post one"));
        assert_eq!(e.link.as_deref(), Some("https://example.com/1"));
        assert_eq!(e.summary, "Hello world", "xhtml content's text");
        assert_eq!(e.author.as_deref(), Some("Ada"));
        assert_eq!(e.published.as_deref(), Some("2026-10-06T07:15:00.000Z"));
    }

    #[test]
    fn rdf_feeds_and_odd_input() {
        let rdf = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <channel><title>Old style</title><link>https://example.org/</link></channel>
  <item rdf:about="https://example.org/x"><title>X</title><link>https://example.org/x</link><dc:date>2026-10-01</dc:date></item>
</rdf:RDF>"#;
        let feed = parse(rdf).unwrap();
        assert_eq!(feed.title, "Old style");
        assert_eq!(feed.items[0].published.as_deref(), Some("2026-10-01T00:00:00.000Z"));
        assert!(parse("<html><body>Not a feed</body></html>").is_err());
        assert!(parse("{\"json\": true}").is_err());
        let long = format!(
            "<rss><channel><item><title>T</title><description>{}</description></item></channel></rss>",
            "word ".repeat(200)
        );
        let summary = &parse(&long).unwrap().items[0].summary;
        assert!(summary.ends_with('…') && summary.chars().count() <= SUMMARY_CHARS + 1, "{summary}");
        assert_eq!(url("javascript:alert(1)"), None);
        assert_eq!(decode_entities("a &amp;amp; b &#8217;s &#x2014; &bogus; &"), "a &amp; b ’s — &bogus; &");
    }
}

#[cfg(test)]
mod probe {
    /// `R1OS_FEED=<file> cargo test -p r1d feed_probe -- --ignored --nocapture`: what the parser makes of a real feed.
    #[test]
    #[ignore]
    fn feed_probe() {
        let path = std::env::var("R1OS_FEED").expect("R1OS_FEED");
        let f = super::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        for i in f.items.iter().take(4) {
            println!("{} | {:?}", i.title, i.image);
        }
    }
}
