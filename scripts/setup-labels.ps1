<#
.SYNOPSIS
    Crée (ou met à jour) les étiquettes des PR et des issues sur GitHub :
    types, zones, suivi et priorité. Conserve les autres étiquettes et leur
    historique. À relancer si la liste change.
#>
. "$PSScriptRoot\common.ps1"

$origin = git -C $PSScriptRoot remote get-url origin
if ($LASTEXITCODE -ne 0 -or -not $origin) { throw 'Remote origin introuvable : aucune étiquette modifiée.' }
$repo = gh repo view $origin.Trim() --json nameWithOwner --jq .nameWithOwner
if ($LASTEXITCODE -ne 0 -or -not $repo) { throw 'Impossible de déterminer le dépôt GitHub : aucune étiquette modifiée.' }

$labels = @(
    # retours des testeurs (issues publiques)
    @{ Name = 'bug'; Color = 'F87171'; Desc = 'Problème signalé par un utilisateur' },
    @{ Name = 'alpha'; Color = '8B5CF6'; Desc = 'Rapport de l''alpha test Aurora' },
    @{ Name = 'question'; Color = '38BDF8'; Desc = 'Question sur le fonctionnement ou l''utilisation' },
    # type de changement (notes de version, .github/release.yml)
    @{ Name = 'nouveauté'; Color = '8B5CF6'; Desc = 'Nouvelle fonctionnalité' },
    @{ Name = 'correctif'; Color = 'F87171'; Desc = 'Correction de bug' },
    @{ Name = 'performance'; Color = '5EEAD4'; Desc = 'Plus rapide, moins de mémoire' },
    @{ Name = 'maintenance'; Color = '6F7BA0'; Desc = 'Refactorisation, dépendances, outillage' },
    @{ Name = 'docs'; Color = 'A78BFA'; Desc = 'Documentation' },
    @{ Name = 'rupture'; Color = 'FB923C'; Desc = 'Casse une compatibilité (réglages, cache, format…)' },
    # zone du viewer
    @{ Name = 'rendu'; Color = '4F46E5'; Desc = 'Moteur de rendu, shaders, GPU' },
    @{ Name = 'avatar'; Color = '818CF8'; Desc = 'Avatars : animations, apparence, mouvements' },
    @{ Name = 'monde'; Color = '4ADE80'; Desc = 'Objets, terrain, environnement, sons du monde' },
    @{ Name = 'réseau'; Color = '070B1F'; Desc = 'Protocole, connexion, assets' },
    @{ Name = 'audio-voix'; Color = 'F472B6'; Desc = 'Son, musique, voix' },
    @{ Name = 'interface'; Color = 'C4B5FD'; Desc = 'Fenêtres, options, ergonomie' },
    @{ Name = 'construction'; Color = 'FCD34D'; Desc = 'Outils de construction et d''édition' },
    @{ Name = 'outillage'; Color = '6F7BA0'; Desc = 'Dépôt GitHub, CI, scripts et environnement de développement' },
    # suivi d’une issue ouverte : un seul état à la fois
    @{ Name = 'à-trier'; Color = 'CBD5E1'; Desc = 'Signalement à analyser et à classer' },
    @{ Name = 'besoin-infos'; Color = 'FCD34D'; Desc = 'Informations ou pièce jointe nécessaires pour poursuivre' },
    @{ Name = 'à-reproduire'; Color = 'FDBA74'; Desc = 'Analyse faite, reproduction du problème encore nécessaire' },
    @{ Name = 'confirmé'; Color = 'F87171'; Desc = 'Problème reproduit ou cause établie par des preuves' },
    @{ Name = 'en-cours'; Color = '8B5CF6'; Desc = 'Prise en charge annoncée avec une tâche ou une PR liée' },
    @{ Name = 'à-valider'; Color = '5EEAD4'; Desc = 'Solution proposée, validation ou fusion encore attendue' },
    # priorité d’une issue : impact constaté, pas une date promise
    @{ Name = 'priorité:haute'; Color = 'DC2626'; Desc = 'Blocage majeur, crash fréquent ou risque de perte de données' },
    @{ Name = 'priorité:normale'; Color = 'FB923C'; Desc = 'Problème ou demande utile sans blocage majeur' },
    @{ Name = 'priorité:basse'; Color = '94A3B8'; Desc = 'Gêne mineure, cosmétique ou amélioration de confort' },
    # motifs de clôture sans correctif
    @{ Name = 'doublon'; Color = 'A1A1AA'; Desc = 'Même problème qu''une autre issue liée dans le commentaire' },
    @{ Name = 'hors-périmètre'; Color = '64748B'; Desc = 'Demande hors du périmètre documenté du projet' }
)
foreach ($l in $labels) {
    Invoke-Checked "label $($l.Name)" { gh label create $l.Name --repo $repo --color $l.Color --description $l.Desc --force }
}
Write-Host "Étiquettes à jour." -ForegroundColor Green
