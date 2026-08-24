# Le manifeste de link — design

**Statut** : spec, validée en discussion le 2026-08-24. Implémente la moitié restante de #338.

**En une phrase** : `link` enregistre ce qu'il a produit et comment le défaire, au moment où il le produit, dans un registre que la réconciliation de la chaîne déclarative consommera ensuite sans être réécrit.

---

## 1. Le problème, mesuré

`link` et `unlink` sont deux implémentations manuscrites de la même opération, écrites séparément. Rien ne les relie : `unlink` ne lit pas le résultat de `link`, il rejoue `linker.generate()` contre la config **courante** et supprime ce qui en sort.

Quatre conséquences confirmées par exécution du binaire réel sur dépôts jetables (2026-08-21) :

- **sur-suppression** — un `.claude/CLAUDE.md` écrit à la main est skippé par `link` puis supprimé par `unlink`, et `.claude/` disparaît avec lui ;
- **orphelin définitif** — un agent retiré de la config laisse son fichier généré pour toujours ; même `link --force` ne le récupère pas, `generate()` ne produisant plus ce chemin ;
- **récursion sur les skills** — des fichiers utilisateur déposés dans `.claude/skills/<nom>/` sont supprimés en silence ;
- **agents déclarés** — comportement identique : le défaut est dans le suivi de fichiers, pas dans la provenance des agents.

#342 a livré une **mitigation** : ne supprimer que si le contenu sur disque correspond à ce que le linker régénère. Elle arrête l'hémorragie mais garde une limite : un fichier généré **puis édité** devient un orphelin, et — cas mesuré — sur la cible `opencode`, `link --model` produit un contenu que `unlink` ne peut pas régénérer à l'identique, donc **tout usage interactif** laisse un fichier non nettoyé.

Le manifeste supprime cette limite : il enregistre l'empreinte du contenu **réellement écrit**, au lieu de la deviner par régénération.

## 2. Ce que le manifeste est

Un **registre de provenance à un seul étage** : pour chaque chemin, ce qui l'a produit, et l'inverse de sa production.

La forme est choisie pour accueillir les étages suivants de la chaîne déclarative sans réécriture. La chaîne cible, décidée en discussion :

```
Config → Gouvernance → Méta-agents → Règles → Agents → link → configs natives
```

`link` en est le **dernier maillon**, pas un mécanisme séparé. Chaque flèche aura besoin de savoir ce qu'elle a produit, pour la même raison. Le champ `produced_by` porte cette extension : aujourd'hui il désigne toujours un agent, demain une règle ou un méta-agent, et la structure ne change pas.

**Ce n'est pas** un graphe de provenance complet. Le construire aujourd'hui serait de la sur-ingénierie pour un bug de perte de données, et la cascade n'existe pas encore. Le coût de l'extensibilité se limite à un champ nommé.

## 3. Format

Fichier : `.armadai/link-manifest.yaml`, un par projet.

```yaml
version: 1
targets:
  claude:
    linked_at: "2026-08-24T09:12:03Z"
    entries:
      - path: .claude/agents/core-specialist.md
        produced_by: { kind: agent, name: core-specialist }
        outcome: created
        digest: "sha256:9f2b…"
      - path: .claude/CLAUDE.md
        produced_by: { kind: coordinator, name: dev-lead }
        outcome: skipped
```

**Champs**

| Champ | Rôle |
|---|---|
| `path` | relatif à la racine du projet. Jamais absolu — un manifeste doit survivre à un déplacement du projet. |
| `produced_by` | ce qui a produit ce fichier. `kind` vaut `agent`, `coordinator`, `skill` ou `prompt` aujourd'hui ; c'est le point d'extension de la chaîne. |
| `outcome` | `created` (`link` a écrit le fichier, ou l'a trouvé déjà présent avec exactement ces octets — voir §12 R6) ou `skipped` (il existait avec un contenu différent, `link` n'y a pas touché). |
| `digest` | empreinte du contenu **écrit par link**. Présent si et seulement si `outcome: created`. |

**L'inverse se déduit de `outcome`**, il n'est pas stocké :

- `created` → l'inverse est *supprimer*, **à condition que le digest corresponde encore**. Sinon le fichier a été édité depuis : on conserve et on le dit.
- `skipped` → l'inverse est *ne rien faire*. C'est le cas qui règle la sur-suppression : `link` n'a rien produit, `unlink` n'a rien à défaire.

Le manifeste est **groupé par cible** parce que `link` et `unlink` opèrent par cible (`--target claude`), et qu'un projet peut être lié à plusieurs.

## 4. Non versionné, avec un repli explicite

Le manifeste décrit un **état local** — quels fichiers existent sur *cette* machine — au même titre qu'un artefact de build. Il n'est donc pas versionné, et `.armadai/` est déjà hors du dépôt dans ce projet.

Conséquence assumée : sur un clone frais, ou après suppression de `.armadai/`, il n'y a pas de manifeste. `unlink` doit alors fonctionner quand même.

**Repli** : sans entrée de manifeste pour une cible, `unlink` retombe sur la garde par contenu de #342 — régénérer et ne supprimer qu'en cas de correspondance — **et l'annonce**, pour que l'utilisateur sache qu'il est en mode dégradé et pourquoi certains fichiers peuvent être conservés.

La mitigation de #342 devient ainsi le mode dégradé du manifeste, pas du code mort à supprimer. C'est ce qui rend les deux chantiers complémentaires plutôt que successifs.

## 5. Comment `link` le produit

L'inverse doit naître **au même endroit que l'effet**. C'est le point que le papier sur la composabilité spatiotemporelle nomme explicitement (§1.2.1, à propos du `deactivate` de VSCode) : séparer la destruction de la création viole la localité de préoccupation et rend le nettoyage invérifiable. ArmadAI faisait pire en re-devinant.

Donc : au point d'écriture unique de `link` (`link.rs:310`), chaque décision produit son entrée — écrite, ou déjà présente avec exactement le même contenu → `created` + digest ; présente avec un contenu différent → `skipped`. Le manifeste est écrit à la fin d'un `link` réussi, en remplacement complet des entrées de cette cible.

`--dry-run` n'écrit pas de manifeste.

## 6. Comment `unlink` le consomme

`unlink --target X` lit les entrées de `X`, et pour chacune applique l'inverse. Rien d'autre : **il ne régénère plus**, donc il ne dépend plus de la config courante, et les orphelins disparaissent — un agent retiré de la config a toujours son entrée dans le manifeste.

Ce qui règle les quatre cas mesurés :

| Cas | Résolution |
|---|---|
| sur-suppression | `outcome: skipped` → inverse noop |
| orphelin | l'entrée existe indépendamment de la config courante |
| récursion skills | seules les entrées enregistrées sont candidates ; un fichier utilisateur n'en a pas |
| `opencode --model` | le digest est celui du contenu écrit, pas d'une régénération |

## 7. Ce que ça prépare, sans le construire

La réconciliation *vue engagée / vue cible* du papier (§4.2, §5.2) consommera ce registre : comparer ce que le manifeste dit avoir produit à ce que la config demande maintenant, et n'appliquer que la différence — au lieu de tout régénérer comme `link` le fait aujourd'hui.

Et une conséquence de structure, notée en discussion : sous un moteur de réconciliation unique, **`unlink` cesse d'être une commande distincte** — c'est « réconcilier vers un état où cette cible n'existe plus ». La divergence entre `link` et `unlink` devient alors impossible par construction, au lieu d'être corrigée à répétition. #341 (appariement du coordinateur divergent entre les deux) disparaît par le même effet.

Rien de tout cela n'est dans le périmètre de cette spec.

## 8. Décisions et leurs raisons

**Le digest plutôt que le contenu.** Stocker le contenu écrit ferait du manifeste une copie complète de la projection. Une empreinte suffit à répondre à la seule question posée : « est-ce toujours ce que j'ai écrit ? »

**⚠️ Choix ouvert — l'algorithme d'empreinte.** Vérifié : le workspace n'a **aucune crate de hachage** (`grep sha2|blake3|digest` sur les `Cargo.toml` : rien). Trois options, à trancher avant l'implémentation :

- **`sha2`** — une dépendance de plus, mais légère, universelle, et le format `sha256:…` se lit sans explication. Le projet gate les dépendances *lourdes* derrière des features ; celle-ci ne l'est pas.
- **Un FNV-1a 64 bits maison** — une dizaine de lignes, zéro dépendance, stable par construction. La résistance cryptographique est inutile ici : on détecte une édition accidentelle, on ne se défend pas contre un adversaire qui forgerait une collision pour faire supprimer un fichier.
- **`std::hash::DefaultHasher`** — **à écarter**. Sa documentation ne garantit pas la stabilité de l'algorithme entre versions de Rust, donc un manifeste écrit aujourd'hui pourrait ne plus se vérifier après une montée de toolchain. Disqualifiant pour un fichier persistant.

Le choix n'affecte que la valeur du champ `digest`, pas la structure. En cas de changement ultérieur, le préfixe (`sha256:`, `fnv1a64:`) permet de reconnaître un manifeste écrit par une version antérieure et de retomber sur le repli de la §4 plutôt que de comparer des empreintes incomparables — **le préfixe est donc obligatoire quel que soit l'algorithme retenu.**

**`outcome` plutôt qu'un inverse stocké.** Deux valeurs suffisent à dériver l'action, et un champ énuméré se lit mieux qu'une commande sérialisée. Si un troisième cas apparaît (un fichier fusionné plutôt qu'écrit ou skippé), il s'ajoute comme valeur.

**Groupé par cible.** Parce que les commandes le sont. Un manifeste plat obligerait à filtrer à chaque opération et rendrait ambigu le cas d'un chemin produit par deux cibles.

**Remplacement complet des entrées d'une cible à chaque `link`.** Une fusion incrémentale demanderait de savoir ce qui a disparu — c'est le travail de la réconciliation, pas du manifeste. Tant que `link` régénère tout, le manifeste enregistre tout.

## 9. Ce que cette spec ne fait pas

- Aucun graphe de provenance multi-étages : `produced_by` est un champ, pas une arête.
- Aucune réconciliation incrémentale : `link` continue de tout régénérer.
- Aucune unification `link`/`unlink` sous un moteur commun.
- Aucun changement du format des fichiers générés.
- Le manifeste n'est pas versionné, donc il ne se partage pas entre machines ni entre membres d'une équipe.

## 10. Auto-revue

**Prémisses vérifiées dans le code**, pas supposées :
- `link` a bien **un seul** point d'écriture (`link.rs:310`, `std::fs::write(path, content)?`), donc « l'inverse naît au même endroit que l'effet » est réalisable sans disperser la logique.
- `.armadai/` est bien dans `.gitignore` (ligne 38), donc « non versionné » est cohérent avec l'existant plutôt qu'une nouvelle règle.
- Aucune crate de hachage dans le workspace → voir le choix ouvert en §8.

**Une ambiguïté volontairement non tranchée** : le comportement de `link` quand un manifeste existe mais qu'un fichier qu'il dit avoir créé a disparu du disque. Deux lectures défendables — le réécrire (l'état cible le demande) ou le signaler (quelqu'un l'a supprimé exprès). Sans la réconciliation, `link` régénère tout de toute façon, donc la question ne se pose pas encore ; elle se posera à ce moment-là et mérite d'être tranchée là, avec le reste.

**Ce que cette spec ne résout pas et qu'on pourrait croire résolu** : elle ne rend pas `link` idempotent. Deux `link` successifs produisent le même résultat aujourd'hui parce que tout est régénéré, pas parce que le manifeste le garantit.

## 11. Amendement sécurité (post-implémentation)

Une revue de code sur l'implémentation initiale a mesuré que `entry.path` était consommé par `unlink` sans validation : une entrée forgée (`../outside/victim.txt`) ou absolue était supprimée si son digest correspondait, et la limite de nettoyage des répertoires vides était elle-même dérivée du même contenu non validé. Un cas sans malice aucune reproduisait le défaut : `link --output ../sibling/out` enregistrait un chemin qui remonte hors du projet — légitime — et `unlink` supprimait `sibling/` **au-dessus** de la racine du projet lors du nettoyage.

Deux ajouts au format, tous les deux dans `TargetManifest` (pas de nouveau champ sur `ManifestEntry` sauf pour l'exception ci-dessous) :

- **`root`** — le répertoire où `link` avait le droit d'écrire pour cette cible (`.claude` par défaut, ou un `--output` explicite, qui peut légitimement pointer hors du projet ou être absolu). `unlink` résout ce chemin (`resolve_real` — canonicalise le plus long préfixe qui existe sur disque, donc suit les liens symboliques réels, plutôt qu'une simple normalisation lexicale ; voir §12 R2) et refuse — en le signalant, jamais en silence — toute entrée ou tout répertoire enregistré qui ne s'y résout pas. C'est la frontière de confiance réelle : elle rend `../sibling/out` valide tout en rejetant `../../etc/anything`.
- **`created_dirs`** — les répertoires que `link` a réellement créés via `create_dir_all`, jamais la racine de la cible elle-même. `unlink` ne devine plus une limite de nettoyage à partir des chemins supprimés : il rejoue l'inverse exact de cette création, du plus profond au moins profond, chacun revalidé contre `root`.

Exception documentée à « `path` n'est jamais absolu » (§3) : quand `root` lui-même est absolu (un `--output` absolu), `path` l'est aussi — cohérent avec `root`, et sans risque puisque la validation de frontière ci-dessus s'applique dans tous les cas.

Conséquence sur §10 : le point d'écriture unique cité (`link.rs:310`) reste vrai pour le contenu des fichiers, mais `create_dir_all` était un second effet sans inverse enregistré — corrigé par `created_dirs`, qui naît au même point que la création elle-même, pas après coup.

## 12. Deuxième amendement (re-revue)

Une seconde revue a mesuré que le premier amendement fermait la moitié la plus faible du problème : `is_trusted` prouve un confinement *à l'intérieur du `root` que le manifeste lui-même déclare* — un `root: "/"` forgé rend cette preuve vide (tout chemin absolu est sous `/`), et reproduit le défaut d'origine en entier sur le binaire déjà corrigé. Mesuré : `outside/victim.txt` supprimé via un `root: "/"` forgé, malgré la garde par entrée.

**R1 (critique) — le `root` du manifeste est maintenant confirmé, jamais cru seul.** `unlink` calcule de toute façon, à l'étape 3, la racine attendue depuis la config/`--output` courants (avant, cette valeur ne servait qu'au repli). `linker::manifest::root_confirmed` compare cette racine calculée à celle déclarée dans le manifeste, résolues l'une et l'autre par `resolve_real` (§ci-dessous). Si elles ne coïncident pas, **toute la cible est refusée** — pas partiellement — et `unlink` retombe sur la garde #342, en l'annonçant : « The link manifest for target '…' declares a root that doesn't match this project's current output directory ». Conséquence de structure : une fois confirmé, c'est **le `root` du manifeste**, pas la valeur `--output`/config recalculée, qui sert ensuite à toutes les opérations de `unlink_from_manifest` — les deux ne peuvent alors diverger que par construction, donc lequel « gagne » n'a plus d'importance pratique, mais c'est bien le premier qui est utilisé mécaniquement.

**R2 (critique en pratique) — la normalisation était lexicale, donc un lien symbolique la traverse sans effort.** `.claude/agents` symlinké vers un répertoire hors projet faisait passer l'entrée `.claude/agents/keys.md` : le *texte* du chemin ne quitte jamais l'arborescence nominale, alors que le fichier réel vit ailleurs. `lexically_normalize` ne mentait pas sur ce qu'elle faisait (sa propre doc l'admet), mais le §11 ci-dessus, lui, ne le disait pas — le code livrait moins que ce que le texte laissait croire. Corrigé par `resolve_real` : canonicalise le plus long préfixe existant du chemin (donc résout les liens symboliques réellement présents sur disque), puis rajoute **toujours** lexicalement le suffixe qui n'existe pas encore — ce rajout lexical n'est pas un repli rare, il a lieu pour tout chemin dont la fin n'existe pas encore, que la racine du projet existe ou non. Le repli sur `resolve` seule (normalisation purement lexicale de bout en bout) ne se produit que si la canonicalisation du plus long préfixe trouvé échoue elle-même (par ex. un problème de permissions) — pas parce que « rien n'existe » : la racine du projet, elle, existe presque toujours. `is_trusted` et `root_confirmed` passent tous les deux par `resolve_real`, jamais par `resolve` seule.

**R3 (important) — fermé par construction pour un `root` forgé, mais pas pour un `created_dirs` qui nomme la vraie racine.** `is_trusted` accepte trivialement un chemin égal à `root` (un chemin se contient toujours lui-même) ; un manifeste dont le `root` est légitime (confirmé par R1) peut donc encore lister la racine elle-même dans `created_dirs`. `unlink` compare maintenant explicitement chaque répertoire enregistré à la racine résolue de la cible, **indépendamment de toute confiance dans le manifeste** — cette garde ne dépend ni de R1 ni de `is_trusted`.

**R4 (important) — un message pouvait décrire un chemin qui n'est pas celui réellement agi.** Un `path` interne du type `a/../b` dont `a` n'existe pas faisait échouer `Path::exists()` sur le chemin brut (la résolution du système de fichiers doit traverser `a` pour appliquer le `..`), et `unlink` rapportait « already absent » même quand `b` existait réellement. Corrigé par le même `resolve_real` : c'est désormais lui, pas `root.join(&entry.path)` brut, qui produit le chemin sur lequel `exists`/lecture/suppression opèrent — R2 et R4 partagent la même correction.

**R5 (mineur, tranché) — un `unlink` qui refuse une entrée sort maintenant en échec.** Un refus est un travail demandé non complété, pas un succès partiel : `unlink_from_manifest` retourne une erreur (après avoir imprimé le résumé) dès qu'au moins une entrée ou un répertoire a été refusé, en plus du repli déclenché par R1 qui reste, lui, un succès dégradé mais complet.

**R6 (important, préexistant depuis `2506f7c`) — `link ; link ; unlink` ne supprimait plus rien.** Un second `link` trouvait chaque fichier déjà présent et l'enregistrait `skipped` — y compris ceux que `link` avait écrits lui-même — et `unlink` les rapportait alors comme « hand-written ». Corrigé : quand `link` trouve un fichier déjà présent **et que son contenu est déjà exactement ce que `link` écrirait**, il l'enregistre `created` avec son empreinte, pas `skipped` ; un contenu qui diffère reste `skipped`.

**Ce correctif élargit ce que `unlink` supprime, pas seulement ce qu'il rapporte.** Dès le *premier* `link`, un fichier écrit à la main mais dont les octets sont déjà, par coïncidence, exactement ceux que `link` générerait est désormais enregistré `created` — donc `unlink` le supprimera, là où il le conservait avant ce correctif. Jugé acceptable : un tel fichier est par définition reproductible par `link` (le regénérer donnerait le même résultat), et l'alternative — le traiter comme `skipped` — est précisément ce qui rend `unlink` inerte après tout re-link, y compris quand rien n'a jamais été modifié à la main. Ce n'est donc pas seulement la véracité du champ `outcome` qui est en jeu, mais le périmètre réel de ce qu'`unlink` reprend.

**Deux endroits où le premier amendement énonçait une intention plutôt que le code, corrigés ici :**
- « la racine de la cible n'est jamais un répertoire à supprimer » n'était garanti qu'à l'**écriture** (`link` ne l'ajoute jamais à `created_dirs`) — rien, côté lecture, n'empêchait un manifeste de la nommer quand même dans `created_dirs`. R3 ajoute la garde côté lecture, indépendante de la confiance dans le manifeste.
- « `created_dirs` rejoue l'inverse exact de la création » n'est vrai que pour un répertoire resté vide : `unlink` ne supprime un répertoire enregistré que s'il existe encore **et** qu'il est vide au moment du nettoyage ; un contenu étranger déposé depuis le laisse intact plutôt que de forcer une suppression récursive. « Exact » décrit la liste des répertoires visés, pas une garantie de suppression inconditionnelle.

