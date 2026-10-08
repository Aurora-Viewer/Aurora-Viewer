<#
.SYNOPSIS
    Crée (ou met à jour) les étiquettes des PR sur GitHub : un type (qui range
    les notes de version) et une zone. Supprime les étiquettes par défaut de
    GitHub. À relancer si la liste change.
#>
. "$PSScriptRoot\common.ps1"

$labels = @(
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
    @{ Name = 'construction'; Color = 'FCD34D'; Desc = 'Outils de construction et d''édition' }
)
foreach ($l in $labels) {
    Invoke-Checked "label $($l.Name)" { gh label create $l.Name --color $l.Color --description $l.Desc --force }
}
$defaults = 'bug', 'documentation', 'duplicate', 'enhancement', 'good first issue', 'help wanted', 'invalid', 'question', 'wontfix'
foreach ($d in $defaults) { gh label delete $d --yes 2>$null | Out-Null }
Write-Host "Étiquettes à jour." -ForegroundColor Green
