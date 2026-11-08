// Mini-proxy de réécriture CSS (version Rust).
// Récupère la page `target`, supprime les éléments correspondant à `remove_css`,
// injecte `inject_css` dans <head> et sert le résultat sur http://localhost:`port`.
// La page /articles liste les articles du flux RSS `feed_url`.
//
// Réglages lus dans config.toml (ou le fichier indiqué par CONFIG) :
//   CONFIG=autre.toml cargo run --release

// Modules du projet : la page /articles (src/articles.rs) et la bannière de démarrage (src/banner.rs).
mod articles;
mod banner;

use std::{env, fs, sync::Arc};

// Désérialisation du fichier TOML vers une structure Rust.
use serde::Deserialize;

// Serveur HTTP : extraction de l'état partagé, types HTTP, réponses et routage.
use axum::{
    Router,
    extract::State,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
// Réécriture du HTML à la volée, avec des sélecteurs CSS.
use lol_html::{RewriteStrSettings, element, html_content::ContentType, rewrite_str};

/// Contenu brut du fichier de configuration, tel qu'écrit dans config.toml.
///
/// - `default` : une clé absente prend la valeur vide de son type
///   ("" pour String, 0 pour u16, [] pour Vec).
/// - `deny_unknown_fields` : une clé inconnue (faute de frappe) est une erreur.
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    /// Scheme du proxy
    scheme: String,
    /// Host du proxy.
    host: String,
    /// Port d'écoute du proxy.
    port: u16,
    /// Site relayé par le proxy (schéma + hôte, sans chemin).
    target: String,
    /// Adresse du flux RSS affiché sur /articles.
    feed_url: String,
    /// CSS ajouté à la fin du <head> de chaque page HTML relayée.
    inject_css: String,
    /// Sélecteurs CSS des éléments à retirer du HTML.
    remove_css: Vec<String>,
}

/// Lit et analyse le fichier TOML situé à `path`.
/// L'erreur renvoyée indique le fichier en cause et, pour le TOML, la ligne fautive.
fn load_config(path: &str) -> Result<FileConfig, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("lecture de {path} : {e}"))?;
    toml::from_str(&text).map_err(|e| format!("{path} : {e}"))
}

/// État partagé entre toutes les requêtes (enveloppé dans un `Arc` dans `main`).
struct Config {
    scheme: String,
    host: String,
    port: u16,
    target: String,
    feed_url: String,
    // Logo du site source, trouvé au premier affichage de /articles.
    feed_logo: std::sync::Mutex<Option<String>>,
    inject_css: String,
    // Sélecteurs CSS des éléments à retirer du HTML (vide = aucun).
    remove: Vec<String>,
    // Client HTTP réutilisé pour toutes les requêtes sortantes (garde les connexions ouvertes).
    client: reqwest::Client,
}

/// Réécrit une page HTML : ajoute `css` dans <head> et retire les éléments
/// correspondant aux sélecteurs de `remove`.
/// Renvoie une erreur si un sélecteur est invalide ou si la réécriture échoue.
fn rewrite(html: &str, css: &str, remove: &[String]) -> Result<String, String> {
    // Balise <style> ajoutée ; l'attribut permet de la repérer dans le code source de la page.
    let style = format!(r#"<style data-injected-by="proxy">{css}</style>"#);

    // Premier gestionnaire : à la rencontre de <head>, ajoute le <style> à la fin de son contenu.
    let mut settings =
        RewriteStrSettings::new().append_element_content_handler(element!("head", |el| {
            el.append(&style, ContentType::Html);
            Ok(())
        }));
    // Un gestionnaire par sélecteur à supprimer : l'élément et tout son contenu sont retirés.
    for selector in remove {
        // element! panique sur un sélecteur invalide : on le valide d'abord.
        selector
            .parse::<lol_html::Selector>()
            .map_err(|e| format!("« {selector} » : {e}"))?;
        settings = settings.append_element_content_handler(element!(selector.as_str(), |el| {
            el.remove();
            Ok(())
        }));
    }

    rewrite_str(html, settings).map_err(|e| e.to_string())
}

/// Gestionnaire de toutes les routes autres que /articles.
/// Relaie la requête vers `target` en conservant le chemin et les paramètres,
/// puis réécrit la réponse si c'est du HTML.
async fn proxy(State(cfg): State<Arc<Config>>, uri: Uri) -> Response {
    // Ex. : /a/b.php?x=1 sur le proxy -> {target}/a/b.php?x=1
    let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let url = format!("{}{}", cfg.target.trim_end_matches('/'), path);

    // Site injoignable (DNS, connexion refusée, TLS…) -> 502 Bad Gateway.
    let upstream = match cfg.client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("Erreur proxy : {e}")).into_response(),
    };
    // On renvoie le même code HTTP que le site d'origine (200, 404…).
    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    // Seul le HTML est modifié ; le reste est relayé tel quel.
    if !content_type.contains("text/html") {
        return match upstream.bytes().await {
            Ok(body) => (status, [(header::CONTENT_TYPE, content_type)], body).into_response(),
            Err(e) => (StatusCode::BAD_GATEWAY, format!("Erreur proxy : {e}")).into_response(),
        };
    }

    // HTML : lecture complète du corps, réécriture, puis envoi en UTF-8.
    let html = match upstream.text().await {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("Erreur proxy : {e}")).into_response(),
    };
    match rewrite(&html, &cfg.inject_css, &cfg.remove) {
        Ok(out) => (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            out,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Erreur de réécriture : {e}"),
        )
            .into_response(),
    }
}

/// Point d'entrée : charge la config, vérifie les sélecteurs, ouvre le port,
/// affiche la bannière puis lance le serveur.
#[tokio::main]
async fn main() {
    // Chemin du fichier de config : variable CONFIG, sinon config.toml dans le dossier courant.
    let config_path = env::var("CONFIG").unwrap_or_else(|_| "config.toml".into());
    let file = match load_config(&config_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Configuration invalide : {e}");
            std::process::exit(1);
        }
    };
    let port = file.port;
    // Passage de la config brute à l'état partagé utilisé par les gestionnaires.
    let cfg = Arc::new(Config {
        scheme: file.scheme,
        host: file.host,
        port: file.port,
        target: file.target,
        feed_url: file.feed_url,
        feed_logo: Default::default(),
        inject_css: file.inject_css,
        // Sélecteurs nettoyés : espaces retirés, entrées vides ignorées.
        remove: file
            .remove_css
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        client: reqwest::Client::builder()
            .user_agent(concat!("css-proxy/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap(),
    });

    // Vérifie les sélecteurs au démarrage plutôt qu'à la première requête.
    if let Err(e) = rewrite("<html></html>", "", &cfg.remove) {
        eprintln!("Sélecteur remove_css invalide : {e}");
        std::process::exit(1);
    }

    // Écoute sur toutes les interfaces (0.0.0.0) ; échec typique : port déjà utilisé.
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Impossible d'écouter sur le port {port} : {e}");
            std::process::exit(1);
        }
    };
    // Affichée une fois le port ouvert, pour ne l'afficher que si le proxy démarre vraiment.
    banner::print(port, &config_path, &cfg.target, &cfg.feed_url, &cfg.remove);

    // Routage : /articles a sa propre page, tout le reste passe par le proxy.
    let app = Router::new()
        .route("/articles", get(articles::page))
        .fallback(proxy)
        .with_state(cfg);
    axum::serve(listener, app).await.unwrap();
}
