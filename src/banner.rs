// Bannière affichée dans le terminal au démarrage.
use std::io::{stdout, IsTerminal};

// Logo « CSS PROXY » sur deux lignes, en caractères de bloc Unicode (▀ ▄ █).
// Chaque lettre fait 3 colonnes ; les deux lignes doivent garder la même largeur.
const LOGO: [&str; 2] = [
    "█▀▀ █▀▀ █▀▀   █▀█ █▀█ █▀█ ▀▄▀ █ █",
    "█▄▄ ▄▄█ ▄▄█   █▀▀ █▀▄ █▄█ ▄▀▄  █ ",
];

/// Affiche le logo, la version et un résumé des réglages actifs.
pub fn print(port: u16, config_path: &str, target: &str, feed_url: &str, remove: &[String]) {
    // Couleurs ANSI seulement dans un vrai terminal (pas dans un fichier ou un pipe).
    let color = stdout().is_terminal();
    // Entoure `s` du code ANSI `code` (1 = gras, 2 = atténué, 36 = cyan), puis réinitialise (0).
    let paint = |code: &str, s: &str| if color { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_owned() };
    // Libellé atténué, complété à 12 caractères pour aligner les valeurs en colonne.
    let label = |s: &str| paint("2", &format!("{s:<12}"));

    println!();
    for line in LOGO {
        println!("  {}", paint("1;36", line));
    }
    // Version lue dans Cargo.toml au moment de la compilation.
    println!("  {}", paint("2", &format!("v{}", env!("CARGO_PKG_VERSION"))));
    println!();
    println!("  {}{}", label("Proxy"), paint("1", &format!("http://localhost:{port}")));
    println!("  {}{}", label("Articles"), paint("1", &format!("http://localhost:{port}/articles")));
    println!("  {}{target}", label("Cible"));
    println!("  {}{feed_url}", label("Flux RSS"));
    println!(
        "  {}{}",
        label("Suppression"),
        if remove.is_empty() { "aucune".into() } else { remove.join(", ") }
    );
    println!("  {}{config_path}", label("Config"));
    println!();
    println!("  {}", paint("2", "Ctrl+C pour arrêter"));
    println!();
}
