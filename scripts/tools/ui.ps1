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
    ink = 'E8EDFB'; muted = '9AA6C6'; muted_dim = '6F7BA0'; bar = '1C1C1C'; raised = '3A3A3A'
    success = '4ADE80'; warn = 'FB923C'; danger = 'F87171'; amber = 'FCD34D'
}
$BasicColors = @{
    violet = 'Magenta'; indigo = 'Blue'; teal = 'Cyan'; violet_light = 'Magenta'
    ink = 'White'; muted = 'Gray'; muted_dim = 'DarkGray'; bar = 'Black'; raised = 'DarkGray'
    success = 'Green'; warn = 'Yellow'; danger = 'Red'; amber = 'Yellow'
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
        Write-Host $Text -ForegroundColor $BasicColors[$Color] -BackgroundColor $(switch ($Background) { 'bar' { 'Black' } 'raised' { 'DarkGray' } default { 'DarkMagenta' } }) -NoNewline:$NoNewline
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

function Set-CursorVisible([bool]$Visible) {
    if (-not $Interactive) { return }
    try { [Console]::CursorVisible = $Visible } catch { }
}

function Clear-Screen { if ($Interactive) { Clear-Host; Set-CursorVisible $false } }

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
    Write-Color ('  ' + 'Item'.PadRight($w1) + 'Found'.PadRight($w2) + $(if (-not $Summary) { 'What I do' })) muted
    Write-Rule $width
    foreach ($c in $Checks) {
        if ($Summary -and $c.State -eq 'always') { continue }
        $found = switch ($c.State) { 'todo' { '• missing' } 'always' { '-' } default { "• $($c.Found)" } }
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
# whatever $FooterStatus adds in between: a script block returning text, or
# coloured segments @(@('text', 'color'), …).
$FooterStatus = $null

function Write-Footer([string]$Hints) {
    if (-not $Interactive) { return }
    $raw = $Host.UI.RawUI
    $w = $raw.WindowSize.Width - 1
    $y = $raw.WindowPosition.Y + $raw.WindowSize.Height - 1
    if ($w -lt 20 -or $y -lt $raw.CursorPosition.Y) { return }
    $status = @(if ($FooterStatus) { & $FooterStatus })
    # one segment comes unrolled: ('text', 'color'); plain text: one muted segment
    if ($status.Count -and $status[0] -isnot [array]) {
        if ($status.Count -eq 2 -and $Palette.ContainsKey([string]$status[1])) { $status = ,@($status[0], $status[1]) }
        else { $status = ,@(($status -join ' '), 'muted') }
    }
    $segments = @($status) + @(, @("  $(Get-Date -Format 'HH:mm:ss') ", 'muted'))
    $left = " $Hints"
    # On narrow windows keep the clock, dropping status segments that do not fit.
    $rightLength = ($segments | ForEach-Object { ([string]$_[0]).Length } | Measure-Object -Sum).Sum
    $start = 0
    while ($start -lt $segments.Count - 1 -and $rightLength -gt $w - 1) {
        $rightLength -= ([string]$segments[$start][0]).Length
        $start++
    }
    $gap = $w - $left.Length - $rightLength
    if ($gap -lt 1) { $left = $left.Substring(0, [Math]::Max(0, $left.Length + $gap - 1)); $gap = 1 }
    $saved = $raw.CursorPosition
    $raw.CursorPosition = New-Object Management.Automation.Host.Coordinates 0, $y
    Write-Color ($left + (' ' * $gap)) muted -Background bar -NoNewline
    for ($i = $start; $i -lt $segments.Count; $i++) { $s = $segments[$i]; Write-Color ([string]$s[0]) $s[1] -Background bar -NoNewline }
    $raw.CursorPosition = $saved
}

# --- Keyboard ---------------------------------------------------------------

# The window size, as text that compares with -eq ('120x30').
function Get-WindowSize {
    $s = $Host.UI.RawUI.WindowSize
    "$($s.Width)x$($s.Height)"
}

# Next key, redrawing the footer every second while waiting; $TickHook (a
# script block) also runs every second (GitHub notifications…). $KeySource (a
# script block returning ConsoleKeyInfo) replaces the keyboard in tests.
# With -Size (the Get-WindowSize the caller drew for): once the window has
# another size and keeps it 200 ms, returns a key 'Resize' (to redraw all:
# the console has reflowed the text, nothing is where it was).
$KeySource = $null
$TickHook = $null
function Read-Key([string]$Hints, [string]$Size) {
    Set-CursorVisible $false
    if ($KeySource) { return & $KeySource }
    $last = [DateTime]::MinValue
    $seen = Get-WindowSize
    $seenAt = [DateTime]::MinValue
    while (-not [Console]::KeyAvailable) {
        $now = Get-WindowSize
        if ($now -ne $seen) { $seen = $now; $seenAt = Get-Date }
        $settled = ((Get-Date) - $seenAt).TotalMilliseconds -ge 200
        if ($Size -and $now -ne $Size -and $settled) { return [pscustomobject]@{ Key = 'Resize'; KeyChar = [char]0 } }
        if ($settled -and ((Get-Date) - $last).TotalMilliseconds -ge 1000) {
            if ($TickHook) { & $TickHook }
            Write-Footer $Hints
            $last = Get-Date
        }
        Start-Sleep -Milliseconds 50
    }
    [Console]::ReadKey($true)
}

function Wait-Back {
    Write-Host ''
    if (-not $Interactive -and -not $KeySource) { return }
    Write-Color '  Esc, Backspace or Enter: back to the menu' muted_dim
    do { $k = Read-Key 'Esc back' } until ($k.Key -in 'Escape', 'Backspace', 'Enter', 'Spacebar')
}

# Yes / no on one key: Y or Enter is yes, N, Esc or Backspace is no.
function Read-YesNo([string]$Question) {
    Write-Host ''
    Write-Color "  $Question " ink -NoNewline
    Write-Color '[Y/n] ' violet_light -NoNewline
    if (-not $Interactive -and -not $KeySource) {
        $a = Read-Host
        return -not ($a -match '^\s*(n|non|no)\s*$')
    }
    while ($true) {
        $k = Read-Key 'Y yes · N or Esc no'
        if ($k.Key -in 'Enter' -or $k.KeyChar -in 'y', 'Y') { Write-Color 'yes' success; return $true }
        if ($k.Key -in 'Escape', 'Backspace' -or $k.KeyChar -in 'n', 'N') { Write-Color 'no' muted; return $false }
    }
}

# A line of text (a path…); empty means cancel.
function Read-Text([string]$Prompt) {
    Write-Host ''
    Write-Color "  $Prompt" ink
    Write-Color '  > ' violet -NoNewline
    Set-CursorVisible $true
    try { (Read-Host).Trim().Trim('"') } finally { Set-CursorVisible $false }
}

# --- Menu -------------------------------------------------------------------
# Items: @{ Key = '1'; Label = '…'; Hint = '…' }. Arrows move, Enter or the
# item's key chooses, Esc or Backspace goes back ($null). $Header draws what
# sits above the menu: everything is redrawn from it when the window is
# resized, so a menu with something above it gives it as $Header.
function Show-Menu([string]$Title, [object[]]$Items, [scriptblock]$Header, [int]$Selected = 0) {
    if (-not $Items.Count) { return $null }
    $hints = '↑↓ choose · Enter confirm · Esc back'
    if (-not $Interactive -and -not $KeySource) {
        if ($Header) { & $Header }
        Write-Title $Title
        foreach ($it in $Items) { Write-Color "    $($it.Key)  $($it.Label)" ink }
        $a = Read-Text 'Your choice (empty: back):'
        return $Items | Where-Object { $_.Key -eq $a } | Select-Object -First 1
    }
    $cursorVisible = $true
    if ($Interactive) { try { $cursorVisible = [Console]::CursorVisible } catch { } }
    Set-CursorVisible $false
    try {
        $sel = [Math]::Max(0, [Math]::Min($Selected, $Items.Count - 1))
        $first = 0
        # hints start in one column, after the longest label
        $column = [Math]::Max(40, ($Items | ForEach-Object { "  $($_.Key)  $($_.Label)".Length + 2 } | Measure-Object -Maximum).Maximum)
        $natural = [Math]::Max(60, ($Items | ForEach-Object { $column + $_.Hint.Length } | Measure-Object -Maximum).Maximum)
        # Every line has the same width, the selected one or not: a shorter line
        # would leave the end of the violet bar on screen when the selection moves.
        $draw = {
            if ($sel -lt $first) { $first = $sel }
            if ($sel -ge $first + $rows) { $first = $sel - $rows + 1 }
            for ($i = $first; $i -lt [Math]::Min($Items.Count, $first + $rows); $i++) {
                if ($Interactive) { $Host.UI.RawUI.CursorPosition = New-Object Management.Automation.Host.Coordinates 0, ($top + $i - $first) }
                $it = $Items[$i]
                $marker = if ($i -eq $sel) { '›' } else { ' ' }
                $label = "$marker $($it.Key)  $($it.Label)".PadRight($column)
                $text = $label + $it.Hint
                if ($text.Length -gt $width) { $text = $text.Substring(0, $width - 1) + '…' }
                $text = $text.PadRight($width)
                Write-Color '  ' -NoNewline
                if ($i -eq $sel) {
                    Write-Color $text ink -Background violet -NoNewline:$Interactive
                } else {
                    $keyEnd = [Math]::Min(5, $text.Length)
                    $labelEnd = [Math]::Min($label.Length, $text.Length)
                    Write-Color $text.Substring(0, $keyEnd) violet_light -NoNewline
                    Write-Color $text.Substring($keyEnd, $labelEnd - $keyEnd) ink -NoNewline
                    Write-Color $text.Substring($labelEnd) muted_dim -NoNewline:$Interactive
                }
            }
            if ($Interactive) { $Host.UI.RawUI.CursorPosition = New-Object Management.Automation.Host.Coordinates 0, ($top + $rows) }
        }
        # all of it (dot-sourced: sets $top, $width and $size here)
        $full = {
            if ($Header) { & $Header }
            Set-CursorVisible $false
            Write-Title $Title
            Write-Host ''
            $size = if ($Interactive) { Get-WindowSize } else { '' }
            # no line wider than the window: a wrapped line shifts all the others
            $width = if ($Interactive) { [Math]::Max(1, [Math]::Min($natural, $Host.UI.RawUI.WindowSize.Width - 3)) } else { $natural }
            $top = if ($Interactive) { $Host.UI.RawUI.CursorPosition.Y } else { 0 }
            $rows = $Items.Count
            if ($Interactive) {
                $raw = $Host.UI.RawUI
                $available = $raw.WindowPosition.Y + $raw.WindowSize.Height - 2 - $top
                # A long preceding transcript must not push the choices off screen.
                # Reserve the footer and a line after the menu; paginate tall lists.
                if ($available -lt [Math]::Min(3, $Items.Count)) {
                    Clear-Screen
                    Set-CursorVisible $false
                    $caption = "  » $Title"
                    if ($caption.Length -gt $width + 2) { $caption = $caption.Substring(0, $width + 1) + '…' }
                    Write-Color $caption violet_light
                    $top = $raw.CursorPosition.Y
                    $available = $raw.WindowPosition.Y + $raw.WindowSize.Height - 2 - $top
                }
                $rows = [Math]::Max(1, [Math]::Min($Items.Count, $available))
            }
            . $draw
        }
        . $full
        while ($true) {
            Write-Footer $hints
            $k = Read-Key $hints $size
            $choice = $null
            switch ($k.Key) {
                'Resize' { Clear-Screen; . $full }
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
            if ($k.Key -ne 'Resize') { . $draw }
        }
    } finally { Set-CursorVisible $cursorVisible }
}
