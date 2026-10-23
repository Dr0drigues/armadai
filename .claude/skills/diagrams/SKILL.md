---
name: diagrams
description: "Quand et comment dessiner un diagramme qu'on fait valider à Dimitri : un bloc Mermaid dans le plan ou la spec (docs/superpowers/), relu avec armadai graph pour un flow. À charger pour concevoir une structure (crates et modules, une séquence entre link, le gate et Claude Code, un modèle de données, un cycle de vie) et chaque fois qu'une structure doit être validée visuellement."
---
# Diagrammes

Une structure que Dimitri doit approuver s'approuve sur une image, pas sur des paragraphes. En dessiner une chaque fois qu'elle aide une décision ; s'en passer quand elle ne ferait que répéter une phrase.

## Quand

- Une architecture de plus de deux composants, ou qui change (crates, modules, adaptateurs).
- Une interaction entre parties : `link` → plan → écriture → rapport ; une session Claude Code → hook → `armadai __claude-policy-gate`.
- Un modèle de données avec relations (types d'`armadai-model`).
- Un cycle de vie à états (une étape de flow, le manifeste de link).
- Chaque fois que Dimitri doit choisir entre deux conceptions : un diagramme par option.

## Quel type (Mermaid)

| Pour montrer | Mermaid |
|---|---|
| composants et dépendances | `flowchart LR` |
| interactions dans le temps | `sequenceDiagram` |
| données et relations | `erDiagram` ou `classDiagram` |
| états et transitions | `stateDiagram-v2` |

Une quinzaine de nœuds au plus, sinon couper. Étiqueter chaque flèche par ce qui passe (un appel, un fichier, un événement). Utiliser les noms du code.

## Où

- Dans le document qui porte la décision : le plan (`docs/superpowers/plans/`) ou la spec (`docs/superpowers/specs/`), dans un bloc ```` ```mermaid ````, suivi de quelques phrases qui l'expliquent. Pas de fichier à part : le Markdown est la source.
- Un flow d'ArmadAI ne se redessine pas à la main : `armadai graph <flow>` l'imprime en Mermaid ; le coller tel quel.

## Le faire valider

Rendre à `lead` le chemin du document, ce qu'il faut regarder (« le gate ne lit que `.armadai/project.yaml` ») et la question à trancher, avec des options quand il y a plusieurs conceptions. Un sous-agent ne peut rien demander : `lead` pose la question. Noter la réponse dans le document, datée.
