---
name: code-nautique
description: Coding conventions with a nautical theme for the pirate crew
apply_to:
  - capitaine
  - vigie
  - charpentier
  - cartographe
---
# Code Nautique — Conventions de Bord

## Cap General
- Le code doit etre lisible par tout membre d'equipage, pas seulement par son auteur
- Chaque fonction a une seule responsabilite — un marin, un poste
- Les noms de variables et fonctions doivent etre descriptifs et sans ambiguite

## Cargaison (Gestion des Erreurs)
- Ne jamais ignorer une erreur — c'est comme ignorer une voie d'eau
- Propager les erreurs avec contexte : expliquer POURQUOI l'operation a echoue
- Valider les entrees aux frontieres du systeme (API, CLI, fichiers)

## Grement (Structure du Code)
- Fonctions courtes et focalisees (< 40 lignes)
- Pas plus de 3 niveaux d'indentation — si le code est trop profond, refactorer
- Grouper les imports par origine : stdlib, deps externes, modules internes

## Journal de Bord (Commits & Documentation)
- Commits atomiques : un changement logique par commit
- Messages de commit descriptifs en anglais (conventional commits)
- Documenter le "pourquoi", pas le "quoi" — le code dit deja le quoi

## Vigie (Revue de Code)
- Toute modification passe par une revue avant merge
- Les tests doivent passer avant soumission
- Corriger les warnings du linter — pas de dette technique tolérée
