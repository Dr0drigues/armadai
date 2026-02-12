# DEMO

```sh
cd examples/demo-nextjs-shop
```

1. Init avec le pack pirate-crew

```sh
armadai init --project --pack pirate-crew
```

2. Générer le GEMINI.md

```sh
armadai link --target gemini
```

3. Lancer gemini et demander une revue

```
$ gemini 

"Fais une revue de code du fichier src/app/api/auth/route.ts"
```

4. Ou demander des tests

```
$ gemini

"Écris les tests manquants pour src/lib/auth.ts"
```

5. Ou demander de la doc

```
$gemini

"Génère la documentation d'API pour les routes dans src/app/api/"
```

L'idée : on utilise le projet demo-nextjs-shop (boutique truffée de bugs) comme terrain de jeu pour les agents pirate-crew :

- Vigie — revue de code : doit trouver les 20+ failles (SQL injection, XSS, MD5, eval, race conditions, leaks...)
- Charpentier — tests : doit constater que les tests existants sont bidon et écrire les vrais
- Cartographe — documentation : doit repérer que le README est faux (mauvais framework, mauvaises variables d'env, fausse couverture de tests) et générer la vraie doc