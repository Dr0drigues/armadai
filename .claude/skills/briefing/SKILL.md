---
name: briefing
description: "Forme d'une spec de lot ArmadAI (docs/superpowers/specs/) et règles d'un bon critère d'acceptation. À charger pour transformer une idée en spec, pour affiner une spec après un changement de périmètre, et avant de juger un travail contre sa spec."
---
# Briefing : la spec d'un lot

Une spec dit pourquoi le lot existe, ce qu'il fait et ne fait pas, et comment on reconnaît qu'il est tenu. C'est la référence contre laquelle chaque étape est jugée : une spec vague fait de chaque étape suivante une supposition.

## Où

`docs/superpowers/specs/<AAAA-MM-JJ>-<sujet>-design.md`, en français. Une seule spec par lot : on l'affine en place, on n'en ouvre pas une deuxième.

## Forme

```markdown
# <titre>

## 1. Intention
Deux à quatre phrases : le problème, pour qui, ce qu'il coûte aujourd'hui, ce que Dimitri a validé (avec la date).

## 2. Objectifs, non-objectifs, anti-objectifs
Objectifs numérotés. Non-objectifs : ce que ce lot ne fait pas exprès. Anti-objectifs : ce qu'il ne doit surtout pas produire.

## 3. Contraintes
Seulement celles qui existent : compatibilité, cibles, format, CI, règles de `armadai-rules` qui pèsent sur le lot.

## 4. Conception
Ce qui change, crate par crate ; les formats et messages visibles par l'utilisateur.

## 5. Critères d'acceptation
1. <un comportement observable, vérifiable par oui ou non>

## 6. Risques
Ce qui peut mal tourner et comment on le verra.

## Questions ouvertes
Ce que Dimitri n'a pas encore tranché.
```

## Bons critères d'acceptation

- Observables et vérifiables par quelqu'un qui n'a pas écrit le code : une commande et sa sortie, un code retour, un fichier écrit ou non. « Rapide » n'est pas un critère.
- Un comportement chacun, numérotés : `validator` rapporte par critère.
- Écrits du côté de l'utilisateur d'`armadai` (ce qu'il tape, voit, obtient), pas du côté du code.
- Peu nombreux : au-delà d'une dizaine, le lot est à couper.
- Un essai réel avec Dimitri en fait partie quand le lot change ce qu'il voit.

## Poser les questions

Une question à la fois, celle dont la réponse change le plus la spec : le problème, puis ce qui compte comme réussi, puis ce qui est hors périmètre, puis les contraintes réelles. Proposer 2 à 4 options concrètes avec une recommandation, et laisser place à une autre réponse. Ne jamais combler un manque par une supposition : le noter sous « Questions ouvertes ».

En sous-agent (`analyst`), on ne peut rien demander : la question va à `lead` sous « Questions pour Dimitri » ; `lead` la pose et renvoie la réponse mot pour mot.

## Affiner après un changement de périmètre

Partir de ce qui a changé (le rapport de `validator`, les mots de Dimitri), mettre à jour les sections concernées, dater le changement dans l'intention, et dire quelles étapes déjà faites sont touchées (plan, tâches).
