# Aurora Tools: colours, layout and keyboard (dot-sourced by aurora-tools.ps1).

# True colours when the terminal understands them, the 16 console colours
# otherwise, plain text when the output is redirected. Interactive: keys are
# read one by one (arrows, Enter, Esc); otherwise answers come as lines.
$Vt = $Host.UI.SupportsVirtualTerminal -and -not [Console]::IsOutputRedirected
$Interactive = -not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected
$Esc = [char]27

# Aurora palette (docs/BRANDING.md).
$Palette = @{
    violet = '8B5CF6'; indigo = '4F46E5'; teal = '5EEAD4'; violet_light = 'A78BFA'
    ink = 'E8EDFB'; muted = '9AA6C6'; muted_dim = '6F7BA0'; bar = '1C1C1C'
    success = '4ADE80'; warn = 'FB923C'; danger = 'F87171'
}
$BasicColors = @{
    violet = 'Magenta'; indigo = 'Blue'; teal = 'Cyan'; violet_light = 'Magenta'
    ink = 'White'; muted = 'Gray'; muted_dim = 'DarkGray'; bar = 'Black'
    success = 'Green'; warn = 'Yellow'; danger = 'Red'
}

function Get-Ansi([string]$Color, [switch]$Background) {
    $h = $Palette[$Color]
    $rgb = (0, 2, 4 | ForEach-Object { [Convert]::ToInt32($h.Substring($_, 2), 16) }) -join ';'
    "$Esc[$(if ($Background) { 48 } else { 38 });2;${rgb}m"
}

function Write-Color([string]$Text, [string]$Color = 'ink', [switch]$NoNewline, [string]$Background) {
    if ($Vt) {
        $bg = if ($Background) { Get-Ansi $Background -Background } else { '' }
        Write-Host "$bg$(Get-Ansi $Color)$Text$Esc[0m" -NoNewline:$NoNewline
    } elseif ($Background) {
        Write-Host $Text -ForegroundColor $BasicColors[$Color] -BackgroundColor $(if ($Background -eq 'bar') { 'Black' } else { 'DarkMagenta' }) -NoNewline:$NoNewline
    } else {
        Write-Host $Text -ForegroundColor $BasicColors[$Color] -NoNewline:$NoNewline
    }
}

# The aurora of the logo: violet, then indigo, then teal, across the text.
function Write-Aurora([string]$Text) {
    if (-not $Vt) { Write-Host $Text -ForegroundColor Magenta; return }
    $stops = @(@(139, 92, 246), @(79, 70, 229), @(94, 234, 212))
    $out = ''
    for ($i = 0; $i -lt $Text.Length; $i++) {
        $t = if ($Text.Length -gt 1) { $i / ($Text.Length - 1) } else { 0 }
        $seg = [Math]::Min([int][Math]::Floor($t * 2), 1)
        $u = $t * 2 - $seg
        $a = $stops[$seg]; $b = $stops[$seg + 1]
        $c = 0..2 | ForEach-Object { [int]($a[$_] + ($b[$_] - $a[$_]) * $u) }
        $out += "$Esc[38;2;$($c[0]);$($c[1]);$($c[2])m$($Text[$i])"
    }
    Write-Host "$out$Esc[0m"
}

function Clear-Screen { if ($Interactive) { Clear-Host } }

function Show-Logo([string]$Subtitle) {
    Clear-Screen
    Write-Host ''
    Write-Aurora '    ▄▀█ █ █ █▀█ █▀█ █▀█ ▄▀█'
    Write-Aurora '    █▀█ █▄█ █▀▄ █▄█ █▀▄ █▀█'
    Write-Color "    V  I  E  W  E  R    ·  $Subtitle" violet_light
    Write-Host ''
}

function Write-Title([string]$Text) {
    Write-Host ''
    Write-Color '  » ' violet -NoNewline
    Write-Color $Text violet_light
}

function Write-Rule([int]$Width = 96) { Write-Color ('  ' + ('─' * $Width)) muted_dim }

# Table of checks (see checks.ps1): what is there, and what will be done
# unless -Summary.
function Show-Checks($Checks, [string]$Title, [switch]$Summary) {
    $w1 = 30; $w2 = 34
    $width = $w1 + $w2 + $(if ($Summary) { 0 } else { 32 })
    Write-Color "  $Title" ink
    Write-Rule $width
    Write-Color ('  ' + 'Élément'.PadRight($w1) + 'Trouvé'.PadRight($w2) + $(if (-not $Summary) { 'Ce que je fais' })) muted
    Write-Rule $width
    foreach ($c in $Checks) {
        if ($Summary -and $c.State -eq 'always') { continue }
        $found = switch ($c.State) { 'todo' { '• absent' } 'always' { '-' } default { "• $($c.Found)" } }
        if ($found.Length -gt $w2 - 2) { $found = $found.Substring(0, $w2 - 3) + '…' }
        $color = switch ($c.State) { 'ok' { 'success' } 'info' { 'teal' } 'always' { 'muted_dim' } default { 'warn' } }
        Write-Color ('  ' + $c.Item.PadRight($w1)) ink -NoNewline
        if ($Summary) {
            Write-Color $found $color
        } else {
            Write-Color $found.PadRight($w2) $color -NoNewline
            Write-Color $c.Action $(if ($c.State -in 'ok', 'info') { 'muted_dim' } else { 'ink' })
        }
    }
    Write-Rule $width
}

# --- Footer -----------------------------------------------------------------
# The last line of the window: key hints on the left, clock on the right, and
# whatever $FooterStatus (a script block returning text) adds in between.
$FooterStatus = $null

function Write-Footer([string]$Hints) {
    if (-not $Interactive) { return }
    $raw = $Host.UI.RawUI
    $w = $raw.WindowSize.Width - 1
    $y = $raw.WindowPosition.Y + $raw.WindowSize.Height - 1
    if ($w -lt 20 -or $y -lt $raw.CursorPosition.Y) { return }
    $status = if ($FooterStatus) { & $FooterStatus } else { '' }
    $clock = Get-Date -Format 'HH:mm:ss'
    $left = " $Hints"
    $right = "$status  $clock "
    $line = $left + (' ' * [Math]::Max(1, $w - $left.Length - $right.Length)) + $right
    if ($line.Length -gt $w) { $line = $line.Substring(0, $w) }
    $saved = $raw.CursorPosition
    $raw.CursorPosition = New-Object Management.Automation.Host.Coordinates 0, $y
    Write-Color $line muted -Background bar -NoNewline
    $raw.CursorPosition = $saved
}

# --- Keyboard ---------------------------------------------------------------

# Next key, redrawing the footer every second while waiting. $KeySource (a
# script block returning ConsoleKeyInfo) replaces the keyboard in tests.
$KeySource = $null
function Read-Key([string]$Hints) {
    if ($KeySource) { return & $KeySource }
    $last = [DateTime]::MinValue
    while (-not [Console]::KeyAvailable) {
        if (((Get-Date) - $last).TotalMilliseconds -ge 1000) { Write-Footer $Hints; $last = Get-Date }
        Start-Sleep -Milliseconds 50
    }
    [Console]::ReadKey($true)
}

function Wait-Back {
    Write-Host ''
    if (-not $Interactive -and -not $KeySource) { return }
    Write-Color '  Échap, Retour arrière ou Entrée : revenir au menu' muted_dim
    do { $k = Read-Key 'Échap retour' } until ($k.Key -in 'Escape', 'Backspace', 'Enter', 'Spacebar')
}

# Yes / no on one key: O or Enter is yes, N, Esc or Backspace is no.
function Read-YesNo([string]$Question) {
    Write-Host ''
    Write-Color "  $Question " ink -NoNewline
    Write-Color '[O/n] ' violet_light -NoNewline
    if (-not $Interactive -and -not $KeySource) {
        $a = Read-Host
        return -not ($a -match '^\s*(n|non|no)\s*$')
    }
    while ($true) {
        $k = Read-Key 'O oui · N ou Échap non'
        if ($k.Key -in 'Enter' -or $k.KeyChar -in 'o', 'O', 'y', 'Y') { Write-Color 'oui' success; return $true }
        if ($k.Key -in 'Escape', 'Backspace' -or $k.KeyChar -in 'n', 'N') { Write-Color 'non' muted; return $false }
    }
}

# A line of text (a path…); empty means cancel.
function Read-Text([string]$Prompt) {
    Write-Host ''
    Write-Color "  $Prompt" ink
    Write-Color '  > ' violet -NoNewline
    (Read-Host).Trim().Trim('"')
}

# --- Menu -------------------------------------------------------------------
# Items: @{ Key = '1'; Label = '…'; Hint = '…' }. Arrows move, Enter or the
# item's key chooses, Esc or Backspace goes back ($null). $Header draws what
# sits above the menu (redrawn with it).
function Show-Menu([string]$Title, [object[]]$Items, [scriptblock]$Header, [int]$Selected = 0) {
    $hints = '↑↓ choisir · Entrée valider · Échap retour'
    if (-not $Interactive -and -not $KeySource) {
        if ($Header) { & $Header }
        Write-Title $Title
        foreach ($it in $Items) { Write-Color "    $($it.Key)  $($it.Label)" ink }
        $a = Read-Text 'Ton choix (vide : retour) :'
        return $Items | Where-Object { $_.Key -eq $a } | Select-Object -First 1
    }
    $sel = [Math]::Max(0, [Math]::Min($Selected, $Items.Count - 1))
    $width = [Math]::Max(60, ($Items | ForEach-Object { $_.Label.Length + $_.Hint.Length + 12 } | Measure-Object -Maximum).Maximum)
    $draw = {
        for ($i = 0; $i -lt $Items.Count; $i++) {
            $it = $Items[$i]
            $text = ("  $($it.Key)  $($it.Label)").PadRight(40) + $it.Hint
            $text = $text.PadRight($width)
            if ($i -eq $sel) {
                Write-Color '  ' -NoNewline
                Write-Color $text ink -Background violet
            } else {
                Write-Color '  ' -NoNewline
                Write-Color ("  $($it.Key)  ").PadRight(5) violet_light -NoNewline
                Write-Color $it.Label.PadRight(35) ink -NoNewline
                Write-Color $it.Hint muted_dim
            }
        }
    }
    if ($Header) { & $Header }
    Write-Title $Title
    Write-Host ''
    $top = if ($KeySource) { 0 } else { $Host.UI.RawUI.CursorPosition.Y }
    & $draw
    while ($true) {
        Write-Footer $hints
        $k = Read-Key $hints
        $choice = $null
        switch ($k.Key) {
            'UpArrow' { $sel = ($sel - 1 + $Items.Count) % $Items.Count }
            'DownArrow' { $sel = ($sel + 1) % $Items.Count }
            'Home' { $sel = 0 }
            'End' { $sel = $Items.Count - 1 }
            'Enter' { return $Items[$sel] }
            { $_ -in 'Escape', 'Backspace' } { return $null }
            default {
                $choice = $Items | Where-Object { $_.Key -eq [string]$k.KeyChar } | Select-Object -First 1
                if (-not $choice -and $k.KeyChar) {
                    $choice = $Items | Where-Object { $_.Key -eq ([string]$k.KeyChar).ToLower() } | Select-Object -First 1
                }
                if ($choice) { return $choice }
            }
        }
        if (-not $KeySource) {
            $Host.UI.RawUI.CursorPosition = New-Object Management.Automation.Host.Coordinates 0, $top
            & $draw
        }
    }
}
