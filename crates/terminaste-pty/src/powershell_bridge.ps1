$global:__TerminasteInputRevision = 0

function global:__TerminasteCharCount([string]$Text) {
  [regex]::Matches($Text, '[\uD800-\uDBFF][\uDC00-\uDFFF]|[\s\S]').Count
}

function global:__TerminasteReportInput {
  $Line = ''
  $Cursor = 0
  [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$Line, [ref]$Cursor)
  __TerminasteEmit 'input-buffer' @{ text = $Line; cursor = (__TerminasteCharCount $Line.Substring(0, $Cursor)); revision = $global:__TerminasteInputRevision }
}

Set-PSReadLineKeyHandler -Chord 'Ctrl+x,r' -ScriptBlock {
  $global:__TerminasteCollecting = $true
  [Microsoft.PowerShell.PSConsoleReadLine]::RevertLine()
}

Set-PSReadLineKeyHandler -Chord 'Ctrl+x,f' -ScriptBlock {
  if ($global:__TerminasteCollecting) {
    $global:__TerminasteCollecting = $false
    $Line = ''
    $Cursor = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$Line, [ref]$Cursor)
    if ($Line -notmatch '^([0-9a-f]{8})([0-9]+);([0-9]+);([A-Za-z0-9+/=]*)$') { return }
    if ([Convert]::ToInt32($Matches[1], 16) -ne $Line.Length - 8) { return }
    $Point = [int]$Matches[2]
    $global:__TerminasteInputRevision = [long]$Matches[3]
    $Text = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($Matches[4]))
    $Chars = [regex]::Matches($Text, '[\uD800-\uDBFF][\uDC00-\uDFFF]|[\s\S]')
    $Offset = if ($Point -lt $Chars.Count) { $Chars[$Point].Index } else { $Text.Length }
    [Microsoft.PowerShell.PSConsoleReadLine]::Replace(0, $Line.Length, $Text)
    [Microsoft.PowerShell.PSConsoleReadLine]::SetCursorPosition($Offset)
  }
  __TerminasteReportInput
}

Set-PSReadLineKeyHandler -Chord 'Ctrl+x,o' -ScriptBlock {
  $Line = ''
  $Cursor = 0
  [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$Line, [ref]$Cursor)
  $Completion = TabExpansion2 $Line $Cursor
  $Items = @($Completion.CompletionMatches | ForEach-Object {
    $Prefix = $Line.Substring(0, $Completion.ReplacementIndex) + $_.CompletionText
    @{ text = $Prefix + $Line.Substring($Completion.ReplacementIndex + $Completion.ReplacementLength); cursor = (__TerminasteCharCount $Prefix) }
  })
  __TerminasteEmit 'completions' @{ text = $Line; revision = $global:__TerminasteInputRevision; items = $Items }
}

Set-PSReadLineKeyHandler -Chord 'Ctrl+x,h' -ScriptBlock {
  $Line = ''
  $Cursor = 0
  [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$Line, [ref]$Cursor)
  $Entries = [Collections.Generic.List[string]]::new()
  $HistoryPath = (Get-PSReadLineOption).HistorySavePath
  if (Test-Path -LiteralPath $HistoryPath) {
    $Pending = ''
    foreach ($Entry in [IO.File]::ReadLines($HistoryPath)) {
      if ($Entry.EndsWith('`')) { $Pending += $Entry.Substring(0, $Entry.Length - 1) + "`n" }
      else { $Entries.Add($Pending + $Entry); $Pending = '' }
    }
  }
  foreach ($Entry in Get-History) { $Entries.Add($Entry.CommandLine) }
  $Entries.Reverse()
  $Seen = [Collections.Generic.HashSet[string]]::new()
  $Items = @($Entries | Where-Object { $_.StartsWith($Line, [StringComparison]::OrdinalIgnoreCase) -and $Seen.Add($_) } | Select-Object -First 2000 | ForEach-Object { @{ text = $_; cursor = (__TerminasteCharCount $_) } })
  __TerminasteEmit 'completions' @{ text = $Line; revision = $global:__TerminasteInputRevision; items = $Items }
}

Set-PSReadLineKeyHandler -Chord 'Ctrl+x,p' -Function HistorySearchBackward
Set-PSReadLineKeyHandler -Chord 'Ctrl+x,n' -Function HistorySearchForward

if ($env:TERMINASTE_WRAPPERS -and (Test-Path /bin/sh)) {
  foreach ($Program in @('ssh', 'sudo', 'su')) {
    $Handler = { & /bin/sh -c ($env:TERMINASTE_WRAPPERS + "`n" + $Program + ' "$@"') terminaste @args }.GetNewClosure()
    Set-Item -Path "Function:global:$Program" -Value $Handler
  }
}
