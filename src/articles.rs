// Page /articles : liste les articles d'un flux RSS (titre, vignette, résumé).
// Les liens renvoient vers le proxy.
use std::sync::{Arc, Mutex};

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use lol_html::{RewriteStrSettings, element, rewrite_str};
use reqwest::Url;

use crate::Config;

/// Flux RSS analysé : informations du <channel> et liste des articles.
struct Feed {
    /// Nom de la source (<channel><title>).
    title: String,
    /// Site de la source (<channel><link>), utilisé pour le lien et le logo de l'en-tête.
    link: String,
    articles: Vec<Article>,
}

/// Un article du flux (<item>).
struct Article {
    title: String,
    /// Résumé fourni par le flux (<description>).
    description: String,
    /// Adresse de l'article sur le site d'origine.
    link: String,
    /// Date de publication, telle qu'écrite dans le flux (format RFC 2822).
    date: String,
    /// Vignette (Media RSS), si le flux en fournit une.
    image: Option<String>,
}

// MEDIA_NS contient l’espace de noms XML de Media RSS, une extension du format RSS qui sert à décrire des images et des vidéos
const MEDIA_NS: &str = "http://search.yahoo.com/mrss/";

/// Analyse le XML d'un flux RSS 2.0 et en extrait la source et les articles.
fn parse_feed(xml: &str, cfg: &Arc<Config>) -> Result<Feed, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    // Texte du premier enfant direct `name` de `node`, sans espaces autour ("" s'il est absent).
    // Les sections CDATA sont renvoyées comme du texte normal.
    let child_text = |node: roxmltree::Node, name: &str| {
        node.children()
            .find(|c| c.has_tag_name(name))
            .and_then(|c| c.text())
            .unwrap_or("")
            .trim()
            .to_owned()
    };

    // Fait passer un lien par le proxy : chemin et paramètres conservés, ajoutés
    // à l'adresse publique (et à son sous-chemin). None si le lien est invalide.
    let replace_host = |link: &str| {
        let src = Url::parse(link).ok()?;
        let mut url = cfg.public_url.clone();
        url.set_path(&format!("{}{}", cfg.base_path, src.path()));
        url.set_query(src.query());
        Some(url.to_string())
    };

    // <channel> porte les informations de la source ; sans lui, ce n'est pas un flux RSS.
    let channel = doc
        .descendants()
        .find(|n| n.has_tag_name("channel"))
        .ok_or("pas d'élément <channel>")?;

    // Chaque <item> devient un Article, dans l'ordre du flux (le plus récent en premier).
    let articles = doc
        .descendants()
        .filter(|n| n.has_tag_name("item"))
        .map(|item| {
            // Toutes les images <media:content> de l'article, avec leur largeur en pixels.
            // Celles sans `url` ou sans `width` numérique sont ignorées.
            let images: Vec<_> = item
                .descendants()
                .filter(|n| n.has_tag_name((MEDIA_NS, "content")))
                .filter_map(|n| {
                    Some((
                        n.attribute("url")?,
                        n.attribute("width")?.parse::<u32>().ok()?,
                    ))
                })
                .collect();
            // Vignette : la plus petite image d'au moins 400 px, sinon la première.
            let image = images
                .iter()
                .filter(|(_, w)| *w >= 400)
                .min_by_key(|(_, w)| *w)
                .or(images.first())
                .map(|(url, _)| url.to_string());

            let link = child_text(item, "link");
            Article {
                title: child_text(item, "title"),
                description: child_text(item, "description"),
                link: replace_host(&link).unwrap_or_default(),
                date: child_text(item, "pubDate"),
                image,
            }
        })
        .collect();

    Ok(Feed {
        title: child_text(channel, "title"),
        link: child_text(channel, "link"),
        articles,
    })
}

// Cherche l'icône la plus grande déclarée dans le <head> du site,
// sinon /favicon.ico. Le résultat est mis en cache dans `cache`.
async fn site_logo(
    client: &reqwest::Client,
    site: &str,
    cache: &Mutex<Option<String>>,
) -> Option<String> {
    // Déjà trouvé lors d'un affichage précédent : pas de nouvelle requête.
    if let Some(logo) = cache.lock().unwrap().clone() {
        return Some(logo);
    }
    let base = Url::parse(site).ok()?;

    // Meilleure icône trouvée jusqu'ici : (taille en px, href tel qu'écrit dans la page).
    let mut best: Option<(u32, String)> = None;
    // Si la page d'accueil est inaccessible, on passe directement au /favicon.ico.
    if let Ok(html) = async { client.get(base.clone()).send().await?.text().await }.await {
        // lol_html sert ici seulement à parcourir les <link> : le HTML réécrit est ignoré.
        let _ = rewrite_str(
            &html,
            RewriteStrSettings::new().append_element_content_handler(element!(
                "link[rel][href]",
                |el| {
                    let rel = el.get_attribute("rel").unwrap_or_default().to_lowercase();
                    if rel.contains("icon") {
                        // sizes="180x180" -> 180 ; sans taille -> 16 (favicon classique).
                        let size = el
                            .get_attribute("sizes")
                            .and_then(|s| s.split('x').next()?.parse().ok())
                            .unwrap_or(16);
                        if best.as_ref().is_none_or(|(b, _)| size > *b) {
                            best = Some((size, el.get_attribute("href").unwrap()));
                        }
                    }
                    Ok(())
                }
            )),
        );
    }

    let href = best
        .map(|(_, h)| h)
        .unwrap_or_else(|| "/favicon.ico".into());
    // href souvent relatif (/cl/icon.png) : on le complète avec l'adresse du site.
    let logo = base.join(&href).ok()?.to_string();
    *cache.lock().unwrap() = Some(logo.clone());
    Some(logo)
}

/// Échappe un texte pour l'insérer dans du HTML (contenu ou valeur d'attribut entre guillemets).
/// Empêche un titre ou un résumé contenant `<script>` ou `"` de casser la page.
/// `&` est remplacé en premier pour ne pas ré-échapper les entités produites ensuite.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Génère la page HTML complète : en-tête (logo + nom de la source) et grille de cartes.
/// Dans le gabarit `format!`, les accolades CSS sont doublées (`{{ }}`) pour ne pas
/// être prises pour des emplacements de variables.
fn render(feed: &Feed, logo: Option<&str>) -> String {
    // Balise <img> du logo, ou rien si aucun logo n'a été trouvé.
    let logo = logo
        .map(|src| format!(r#"<img class="logo" src="{}" alt="">"#, escape(src)))
        .unwrap_or_default();
    // Une carte <article> par article : toute la carte est un lien (nouvel onglet).
    let cards: String = feed
        .articles
        .iter()
        .map(|a| {
            // loading="lazy" : les images hors écran ne sont chargées qu'au défilement.
            let img = a
                .image
                .as_deref()
                .map(|src| format!(r#"<img src="{}" alt="" loading="lazy">"#, escape(src)))
                .unwrap_or_default();
            format!(
                r#"<article><a href="{link}" target="_blank" rel="noopener">{img}<div><h2>{title}</h2><time>{date}</time><p>{desc}</p></div></a></article>"#,
                link = escape(&a.link),
                title = escape(&a.title),
                date = escape(&a.date),
                desc = escape(&a.description),
            )
        })
        .collect();

    format!(
        r#"<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Articles</title>
<style>
  /* Couleurs en variables, redéfinies pour le thème sombre du système. */
  :root {{ --bg: #f6f6f4; --card: #fff; --text: #1d1d1b; --muted: #6b6b66; --border: #e3e3df; }}
  @media (prefers-color-scheme: dark) {{ :root {{ --bg: #161615; --card: #20201e; --text: #ececea; --muted: #9a9a94; --border: #33332f; }} }}
  * {{ box-sizing: border-box; }}
  body {{ margin: 0; background: var(--bg); color: var(--text); font: 16px/1.5 system-ui, sans-serif; }}
  header {{ max-width: 1100px; margin: 0 auto; padding: 24px 16px 8px; }}
  header h1 {{ margin: 0; font-size: 1.5rem; }}
  header p {{ margin: 4px 0 0; color: var(--muted); font-size: .875rem; }}
  .source {{ display: inline-flex; align-items: center; gap: 12px; color: inherit; text-decoration: none; }}
  .source:hover h1 {{ text-decoration: underline; }}
  .logo {{ width: 48px; height: 48px; border-radius: 10px; object-fit: contain; flex: none; }}
  /* Grille : autant de colonnes de 300 px minimum que la largeur le permet. */
  main {{ max-width: 1100px; margin: 0 auto; padding: 16px; display: grid; gap: 16px; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); }}
  article {{ background: var(--card); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; }}
  article a {{ color: inherit; text-decoration: none; display: block; height: 100%; }}
  article a:hover h2 {{ text-decoration: underline; }}
  img {{ width: 100%; aspect-ratio: 16 / 10; object-fit: cover; display: block; background: var(--border); }}
  article div {{ padding: 12px 14px 16px; }}
  h2 {{ margin: 0 0 4px; font-size: 1.05rem; line-height: 1.3; }}
  time {{ color: var(--muted); font-size: .8rem; }}
  article p {{ margin: 8px 0 0; color: var(--muted); font-size: .9rem; }}
</style>
</head>
<body>
<header>
  <a class="source" href="{link}" target="_blank" rel="noopener">{logo}<h1>{title}</h1></a>
  <p>{count} articles</p>
</header>
<main>{cards}</main>
</body>
</html>"#,
        link = escape(&feed.link),
        title = escape(&feed.title),
        count = feed.articles.len(),
    )
}

/// Gestionnaire de la route /articles.
/// Le flux est relu à chaque affichage ; seul le logo est mis en cache.
/// Flux inaccessible ou illisible -> 502 avec un message d'erreur.
pub async fn page(State(cfg): State<Arc<Config>>) -> Response {
    // Téléchargement du flux : deux échecs possibles (requête, puis lecture du corps).
    let xml = match cfg.client.get(&cfg.feed_url).send().await {
        Ok(r) => match r.text().await {
            Ok(t) => t,
            Err(e) => {
                return (StatusCode::BAD_GATEWAY, format!("Erreur flux : {e}")).into_response();
            }
        },
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("Erreur flux : {e}")).into_response(),
    };
    let feed = match parse_feed(&xml, &cfg) {
        Ok(f) => f,
        Err(e) => {
            return (StatusCode::BAD_GATEWAY, format!("Flux RSS illisible : {e}")).into_response();
        }
    };
    let logo = site_logo(&cfg.client, &feed.link, &cfg.feed_logo).await;
    Html(render(&feed, logo.as_deref())).into_response()
}
