# css-proxy

Mini-proxy HTTP en Rust qui relaie un site et réécrit ses pages HTML à la volée :

- ajoute du CSS à la fin du `<head>` de chaque page ;
- retire du HTML les éléments correspondant à des sélecteurs CSS ;
- sert sur `/articles` une page listant les articles d'un flux RSS, avec des liens qui passent par le proxy.

Les réponses qui ne sont pas du HTML (images, scripts, feuilles de style…) sont relayées sans modification.

## Démarrage rapide

```bash
cp config.sample.toml config.toml
# adapter config.toml, puis :
cargo run --release
```

Le proxy écoute sur toutes les interfaces, sur le port indiqué dans la config :

- `http://localhost:3000/` : le site relayé et réécrit ;
- `http://localhost:3000/articles` : la liste des articles du flux RSS.

Pour utiliser un autre fichier de configuration :

```bash
CONFIG=autre.toml cargo run --release
```

## Configuration

Toute la configuration est dans un fichier TOML, `config.toml` par défaut. Voir [config.sample.toml](config.sample.toml) pour un exemple complet.

| Clé          | Type            | Rôle                                                                 |
|--------------|-----------------|----------------------------------------------------------------------|
| `scheme`     | texte           | Protocole des liens réécrits vers le proxy (`http` ou `https`).     |
| `host`       | texte           | Hôte des liens réécrits vers le proxy.                               |
| `port`       | entier          | Port d'écoute du proxy, repris aussi dans les liens réécrits.        |
| `target`     | texte           | Site relayé (schéma + hôte, sans chemin).                            |
| `feed_url`   | texte           | Adresse du flux RSS affiché sur `/articles`.                         |
| `inject_css` | texte           | CSS ajouté dans un `<style>` à la fin du `<head>`.                   |
| `remove_css` | liste de textes | Sélecteurs CSS des éléments à retirer du HTML.                       |

Quelques points à connaître :

- Une clé absente prend une valeur vide : `""`, `0` ou `[]`. Avec `port` absent, le proxy écoute sur un port aléatoire.
- Une clé inconnue, par exemple une faute de frappe, est refusée au démarrage.
- Les sélecteurs de `remove_css` sont vérifiés au démarrage. Un sélecteur invalide arrête le programme avec un message d'erreur.
- `scheme`, `host` et `port` doivent correspondre à l'adresse sous laquelle le proxy est joignable : c'est elle qui sert à réécrire les liens de `/articles`.

## Docker

```bash
docker build -t css-proxy .
```

```bash
docker run --rm -p 3000:3000 -v ./config.toml:/app/config.toml css-proxy
```

L'image contient une copie du `config.toml` présent au moment du build. Le `-v` permet de la remplacer sans reconstruire l'image. Si tu changes `port`, adapte aussi le `-p`.

## Fonctionnement

- Toute requête autre que `/articles` est relayée vers `target`, avec le même chemin et les mêmes paramètres.
- Le code HTTP du site d'origine est conservé.
- Si le site est injoignable, le proxy répond `502 Bad Gateway`.
- Le HTML est réécrit avec [lol_html](https://crates.io/crates/lol_html), puis renvoyé en UTF-8.
- Le `<style>` injecté porte l'attribut `data-injected-by="proxy"`, ce qui permet de le repérer dans le code source de la page.

## Structure

```
src/
├── main.rs      # config, réécriture HTML, proxy et routage
├── articles.rs  # page /articles (lecture du flux RSS)
└── banner.rs    # bannière affichée au démarrage
```
