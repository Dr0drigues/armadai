# DEMO

```sh
cd examples/demo-nextjs-shop
```

1. Le projet ArmadAI est déjà là

`.armadai/` contient l'équipage du pack `pirate-crew` au format canonique : 4 agents
(`capitaine`, coordinateur, et `vigie`, `charpentier`, `cartographe`), la skill `code-nautique`
et le flow `mission`. Il a été créé par :

```sh
armadai init --pack pirate-crew
```

Pour le recréer depuis le pack : `rm -rf .armadai && armadai init --pack pirate-crew`
(`init --pack` refuse d'écrire dans un projet qui a déjà un `.armadai/project.yaml`).

2. Vérifier le projet et générer la configuration Claude Code

```sh
armadai validate
armadai graph
armadai link --target claude
```

`validate` doit répondre `0 error(s), 0 warning(s)`. `link` écrit `.claude/agents/` (vigie,
charpentier, cartographe), `.claude/skills/code-nautique/` et un bloc `armadai` dans le
`CLAUDE.md` racine de l'exemple : le capitaine y devient la session principale, qui délègue à
ses matelots. Ces fichiers générés sont ignorés par git (`.gitignore` de l'exemple).
`armadai unlink` les retire.

3. Lancer Claude Code et demander une revue

```
$ claude

"Fais une revue de code du fichier src/app/api/auth/route.ts"
```

4. Ou demander des tests

```
$ claude

"Écris les tests manquants pour src/lib/auth.ts"
```

5. Ou demander de la doc

```
$ claude

"Génère la documentation d'API pour les routes dans src/app/api/"
```

L'idée : on utilise le projet demo-nextjs-shop (boutique truffée de bugs) comme terrain de jeu pour les agents pirate-crew :

- Vigie — revue de code : doit trouver les 20+ failles (SQL injection, XSS, MD5, eval, race conditions, leaks...)
- Charpentier — tests : doit constater que les tests existants sont bidon et écrire les vrais
- Cartographe — documentation : doit repérer que le README est faux (mauvais framework, mauvaises variables d'env, fausse couverture de tests) et générer la vraie doc

Le `README.md` de l'exemple est volontairement faux : c'est un appât, ne le corrigez pas.
`.env.example` ne contient que des valeurs factices (`cp .env.example .env` pour lancer l'app).
