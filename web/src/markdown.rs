//! Markdown for news posts and site content, rendered on the server.
//!
//! What a visitor gets is built from the parser's events, not from the author's text:
//! raw HTML is turned into plain text (so `<script>` shows up as words), images become
//! their alt text, and links keep only `http`, `https`, `mailto` and same-site paths. The
//! same function serves the admin preview, so Write/Preview shows exactly what visitors see.

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

/// The longest text the renderer will take; callers enforce their own, smaller limits.
pub const MAX_INPUT: usize = 40_000;

/// Is this link target one visitors may follow?
pub fn safe_url(url: &str) -> bool {
    // Whitespace and control characters can hide a scheme from a check (`java\tscript:`).
    if url.is_empty() || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return lower.split_once("://").is_some_and(|(_, rest)| !rest.is_empty());
    }
    if lower.starts_with("mailto:") {
        return lower.len() > "mailto:".len();
    }
    // A path on this site. "//host" and "/\host" both leave it.
    url.starts_with('/') && !url.starts_with("//") && !url.starts_with("/\\")
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

fn options() -> Options {
    // No ENABLE_HTML-style switch exists to turn raw HTML off in the parser; it is neutralised below.
    Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES
}

/// HTML for `src`. Never contains markup the author typed.
pub fn render(src: &str) -> String {
    let src = truncate(src, MAX_INPUT);
    let mut out = String::with_capacity(src.len() * 2);
    let mut open_links: Vec<bool> = Vec::new();
    let mut image_depth = 0usize;
    let events = Parser::new_ext(src, options()).filter_map(|ev| match ev {
        // Raw HTML becomes text, which the renderer escapes.
        Event::Html(t) | Event::InlineHtml(t) => Some(Event::Text(t)),
        Event::Start(Tag::Image { .. }) => {
            image_depth += 1;
            None
        }
        Event::End(TagEnd::Image) => {
            image_depth = image_depth.saturating_sub(1);
            None
        }
        Event::Start(Tag::Link { dest_url, link_type, .. }) => {
            // <me@example.org> arrives without its scheme.
            let dest_url = if link_type == LinkType::Email { CowStr::from(format!("mailto:{dest_url}")) } else { dest_url };
            let ok = safe_url(&dest_url);
            open_links.push(ok);
            if !ok {
                return None;
            }
            let mut a = String::from("<a href=\"");
            escape(&dest_url, &mut a);
            a.push_str("\" rel=\"nofollow noopener\">");
            Some(Event::Html(CowStr::from(a)))
        }
        Event::End(TagEnd::Link) => match open_links.pop() {
            Some(true) => Some(Event::Html(CowStr::Borrowed("</a>"))),
            _ => None,
        },
        other => Some(other),
    });
    pulldown_cmark::html::push_html(&mut out, events);
    out
}

/// The first `max` characters of what the text says, without any Markdown, for lists.
pub fn excerpt(src: &str, max: usize) -> String {
    let mut text = String::new();
    for ev in Parser::new_ext(truncate(src, MAX_INPUT), options()) {
        match ev {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph) | Event::End(TagEnd::Item) | Event::End(TagEnd::Heading(_))
                if !text.ends_with(' ') && !text.is_empty() =>
            {
                text.push(' ');
            }
            _ => {}
        }
        if text.chars().count() > max + 40 {
            break;
        }
    }
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The host part of a URL like `https://play.example.net:8443/x`, for showing as an address.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://")?.1;
    let auth = rest.split(['/', '?', '#']).next()?;
    let auth = auth.rsplit('@').next()?;
    let host = if let Some(v6) = auth.strip_prefix('[') {
        let end = v6.find(']')?;
        format!("[{}]", &v6[..end])
    } else {
        auth.split(':').next()?.to_string()
    };
    if host.is_empty() || host.chars().any(|c| !(c.is_ascii_alphanumeric() || "-._[]:".contains(c))) {
        return None;
    }
    Some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_markdown_renders() {
        let h = render("# Hi\n\nSome **bold** and a [link](https://example.org/a?b=1&c=2).\n\n- one\n- two");
        assert!(h.contains("<h1>Hi</h1>"));
        assert!(h.contains("<strong>bold</strong>"));
        assert!(h.contains("<a href=\"https://example.org/a?b=1&amp;c=2\" rel=\"nofollow noopener\">link</a>"));
        assert!(h.contains("<li>one</li>"));
    }

    #[test]
    fn raw_html_and_script_tags_come_out_as_text() {
        for src in [
            "<script>alert(1)</script>",
            "hello <script>alert(1)</script> there",
            "<img src=x onerror=alert(1)>",
            "<div onclick=\"x()\">hi</div>",
            "<iframe src=\"https://evil.example\"></iframe>",
            "text <b>bold</b>",
        ] {
            let h = render(src);
            assert!(!h.contains("<script"), "{src} -> {h}");
            assert!(!h.contains("<img"), "{src} -> {h}");
            assert!(!h.contains("<div"), "{src} -> {h}");
            assert!(!h.contains("<iframe"), "{src} -> {h}");
            assert!(!h.contains("<b>"), "{src} -> {h}");
            assert!(h.contains("&lt;"), "{src} -> {h}");
        }
    }

    #[test]
    fn only_safe_links_survive() {
        for bad in [
            "[x](javascript:alert(1))",
            "[x](JaVaScRiPt:alert(1))",
            "[x](data:text/html;base64,AAAA)",
            "[x](vbscript:msgbox(1))",
            "[x](//evil.example/a)",
            "[x](/\\evil.example)",
            "[x](file:///etc/passwd)",
            "[x](<java\tscript:alert(1)>)",
            "[x](<javascript&colon;alert(1)>)",
            "[x](<  javascript:alert(1)>)",
            "[x](ftp://example.org/f)",
        ] {
            let h = render(bad);
            assert!(!h.contains("href"), "{bad} -> {h}");
            assert!(!h.contains("<a"), "{bad} -> {h}");
        }
        let ok = render("[a](http://example.org) [b](https://example.org) [c](mailto:me@example.org) [d](/rules)");
        assert_eq!(ok.matches("<a href=").count(), 4, "{ok}");
        assert_eq!(ok.matches("rel=\"nofollow noopener\"").count(), 4);
        // The link text stays when the link is dropped.
        assert!(render("[click](javascript:alert(1))").contains("click"));
    }

    #[test]
    fn a_quote_in_a_link_cannot_break_out_of_the_attribute() {
        let h = render("[x](https://example.org/\"onmouseover=\"alert(1))");
        assert!(!h.contains("\"onmouseover"), "{h}");
        let h = render("[x](<https://example.org/a\"b'c>)");
        assert!(h.contains("&quot;") && h.contains("&#39;"), "{h}");
    }

    #[test]
    fn email_autolinks_keep_their_link() {
        let h = render("write <me@example.org> now");
        assert!(h.contains("<a href=\"mailto:me@example.org\""), "{h}");
    }

    #[test]
    fn images_become_their_alt_text() {
        let h = render("![a cat](https://evil.example/cat.png) and ![](/x.png)");
        assert!(!h.contains("<img"), "{h}");
        assert!(!h.contains("evil.example"), "{h}");
        assert!(h.contains("a cat"));
    }

    #[test]
    fn long_and_nested_input_is_cut_and_survives() {
        let long = "a".repeat(MAX_INPUT * 2);
        let h = render(&long);
        assert!(h.len() < MAX_INPUT + 100);
        let nested = ">".repeat(30_000);
        let _ = render(&nested);
        let lists = "- ".repeat(20_000);
        let _ = render(&lists);
        let multi = "é".repeat(MAX_INPUT); // two bytes each: the cut must land on a character
        assert!(!render(&multi).is_empty());
    }

    #[test]
    fn excerpts_are_plain_text() {
        let e = excerpt("# Title\n\nSome **bold** words <script>x</script> here.\n\n- a\n- b", 200);
        assert_eq!(e, "Title Some bold words <script>x</script> here. a b");
        let long = excerpt(&"word ".repeat(200), 50);
        assert!(long.ends_with('…') && long.chars().count() <= 51);
    }

    #[test]
    fn the_address_comes_from_the_hosts_name() {
        assert_eq!(host_of("https://play.example.net").as_deref(), Some("play.example.net"));
        assert_eq!(host_of("https://play.example.net:8443/x?y").as_deref(), Some("play.example.net"));
        assert_eq!(host_of("http://u:p@host.test/").as_deref(), Some("host.test"));
        assert_eq!(host_of("http://[::1]:8080/").as_deref(), Some("[::1]"));
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("not a url"), None);
        assert_eq!(host_of("https://bad host/"), None);
    }
}
