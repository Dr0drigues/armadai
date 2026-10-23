---
name: cadre
description: "Le cadre d'un lot ArmadAI : quelles règles du dépôt s'appliquent (skill armadai-rules, CLAUDE.md, .claude/CLAUDE.md, CI), ce qui reste à décider, et la règle de demander à Dimitri avant d'adopter une convention qu'aucune règle ne fixe. À charger avant d'écrire, relire ou planifier du code, et dès qu'une convention, une dépendance ou un format doit être choisi."
---
# Cadre : les règles d'un lot

Le cadre est l'ensemble des règles que le code suit. Il appartient à Dimitri : le dépôt en fixe une partie, Dimitri décide le reste, et aucun agent n'y ajoute rien de lui-même.

## Où

- Les règles décidées : la skill `armadai-rules` (`.armadai/skills/armadai-rules/SKILL.md`), une ligne par règle, chacune avec sa source. Seul `framer` l'écrit.
- Les sources qu'elle cite : `CLAUDE.md`, `.claude/CLAUDE.md`, la CI (`.github/workflows/`), les hooks (`.githooks/`), la mémoire du coordinateur, les décisions datées de Dimitri.
- Le cadre propre à un lot (ce qui s'applique, ce qui est à décider) : la réponse de `framer` à `lead`, reprise par `architect` dans les contraintes globales du plan.

## Ce qui compte comme décidé

- Une règle qu'un outil impose : CI, `clippy -D warnings`, `rustfmt`, le hook `commit-msg`. Source : le fichier.
- Une règle écrite dans `CLAUDE.md` ou `.claude/CLAUDE.md`. Source : le fichier et sa section.
- Une décision de Dimitri, datée. Source : « Dimitri, AAAA-MM-JJ ».
- Une leçon mesurée de la mémoire, avec son fichier.

Une habitude vue dans le code mais imposée par rien est **observée**, pas décidée : la proposer à Dimitri.

## La règle : demander avant d'adopter une convention

Avant d'écrire du code qui dépend d'une convention absente du cadre (un nouveau code de diagnostic, un format de sortie, une dépendance, une forme de message, un emplacement de fichier) :

1. Formuler la question, 2 à 4 options avec ce que chacune implique, une recommandation et ce qui en dépend.
2. En sous-agent, on ne peut rien demander : la rendre à `lead` sous « Questions pour Dimitri » ; `lead` la pose et renvoie la réponse.
3. Enregistrer la réponse (par `framer`) dans `armadai-rules`, avec la source « Dimitri, AAAA-MM-JJ ».
4. Seulement ensuite écrire le code.

Une option évidente reste à choisir par Dimitri. Ne jamais inventer une source, ne jamais enregistrer comme décidé ce que Dimitri n'a pas dit.
