//! HTML → Markdown minimaliste (export des journaux) — port du convertisseur TS.

use regex::Regex;
use std::sync::LazyLock;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(UUID, r"@UUID\[[^\]]+\]\{([^}]*)\}");
re!(HEADING, r"(?is)<h([1-6])[^>]*>(.*?)</h[1-6]>");
re!(LI, r"(?is)<li[^>]*>(.*?)</li>");
re!(BLOCKQUOTE, r"(?is)<blockquote[^>]*>(.*?)</blockquote>");
re!(BR, r"(?i)<br\s*/?>");
re!(HR, r"(?i)<hr\s*/?>");
re!(PDIV_OPEN, r"(?i)<(p|div)[^>]*>");
re!(PDIV_CLOSE, r"(?i)</(p|div|ul|ol|table|thead|tbody)>");
re!(TR, r"(?is)<tr[^>]*>(.*?)</tr>");
re!(TD, r"(?is)<t[hd][^>]*>(.*?)</t[hd]>");
re!(STRONG, r"(?is)<(strong|b)[^>]*>(.*?)</(strong|b)>");
re!(EM, r"(?is)<(em|i)[^>]*>(.*?)</(em|i)>");
re!(CODE, r"(?is)<code[^>]*>(.*?)</code>");
re!(A, r#"(?is)<a[^>]*href="([^"]*)"[^>]*>(.*?)</a>"#);
re!(IMG, r#"(?i)<img[^>]*src="([^"]*)"[^>]*>"#);
re!(TAG, r"<[^>]+>");
re!(BLANKS, r"\n{3,}");

pub fn html_to_markdown(html: &str) -> String {
    let mut md = html.to_string();
    md = UUID.replace_all(&md, "**$1**").into_owned();
    md = HEADING
        .replace_all(&md, |c: &regex::Captures| {
            let level: usize = c[1].parse().unwrap_or(1);
            format!("\n{} {}\n", "#".repeat(level), c[2].trim())
        })
        .into_owned();
    md = LI.replace_all(&md, "\n- $1").into_owned();
    md = BLOCKQUOTE
        .replace_all(&md, |c: &regex::Captures| {
            format!("\n> {}\n", c[1].trim().replace('\n', "\n> "))
        })
        .into_owned();
    md = TR
        .replace_all(&md, |c: &regex::Captures| {
            let cells: Vec<String> = TD
                .captures_iter(&c[1])
                .map(|m| m[1].trim().to_string())
                .collect();
            if cells.is_empty() {
                String::new()
            } else {
                format!("\n| {} |", cells.join(" | "))
            }
        })
        .into_owned();
    md = BR.replace_all(&md, "\n").into_owned();
    md = HR.replace_all(&md, "\n---\n").into_owned();
    md = PDIV_OPEN.replace_all(&md, "\n").into_owned();
    md = PDIV_CLOSE.replace_all(&md, "\n").into_owned();
    md = STRONG.replace_all(&md, "**$2**").into_owned();
    md = EM.replace_all(&md, "*$2*").into_owned();
    md = CODE.replace_all(&md, "`$1`").into_owned();
    md = A.replace_all(&md, "[$2]($1)").into_owned();
    md = IMG.replace_all(&md, "![]($1)").into_owned();
    md = TAG.replace_all(&md, "").into_owned();
    md = md
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    BLANKS.replace_all(&md, "\n\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titres_emphase_listes() {
        let md = html_to_markdown("<h2>Acte VI</h2><p>Un <strong>duel</strong> <em>tendu</em>.</p><ul><li>Riar</li><li>Toydaria</li></ul>");
        assert!(md.contains("## Acte VI"));
        assert!(md.contains("**duel**"));
        assert!(md.contains("*tendu*"));
        assert!(md.contains("- Riar"));
    }

    #[test]
    fn uuid_aplati_et_tableaux() {
        assert_eq!(
            html_to_markdown(
                r#"<p>@UUID[Compendium.world.crits.abc]{Stress mécanique} · Facile</p>"#
            ),
            "**Stress mécanique** · Facile"
        );
        let md = html_to_markdown("<table><tr><th>d100</th><th>Effet</th></tr><tr><td>1-9</td><td>Stress</td></tr></table>");
        assert!(md.contains("| d100 | Effet |"));
        assert!(md.contains("| 1-9 | Stress |"));
    }

    #[test]
    fn entites_et_nettoyage() {
        assert_eq!(
            html_to_markdown("<span>a&nbsp;&amp;&nbsp;b</span>"),
            "a & b"
        );
        assert_eq!(html_to_markdown("<section><p>texte</p></section>"), "texte");
    }
}
