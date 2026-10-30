# `cmds [words]`: the cheat sheet in the shell. Added to every PowerShell the cloud starts
# (terminal_store::ps_hook_encoded); the list is static/commands.json, written to $env:LP_CMDS.
function global:cmds {
  $en = $env:LP_LANG -eq 'en'
  if (-not ($env:LP_CMDS -and (Test-Path -LiteralPath $env:LP_CMDS))) {
    Write-Host $(if ($en) { 'cmds: the list is missing, restart the cloud' } else { 'cmds: список не найден, перезапустите облако' })
    return
  }
  $all = Get-Content -LiteralPath $env:LP_CMDS -Raw -Encoding UTF8 | ConvertFrom-Json
  $words = @($args | ForEach-Object { "$_".ToLower().Replace('ё', 'е') } | Where-Object { $_ })
  $hits = @($all | Where-Object {
    $text = ($_.g, $_.ge, $_.cmd, $_.ru, $_.en -join ' ').ToLower().Replace('ё', 'е')
    -not @($words | Where-Object { -not $text.Contains($_) }).Count
  })
  if (-not $hits.Count) {
    Write-Host $(if ($en) { 'Nothing found. Plain cmds shows everything.' } else { 'Ничего не найдено. Просто cmds покажет всё.' })
    return
  }
  # A phone screen has no room for two columns: the description goes under the command.
  $narrow = $Host.UI.RawUI.WindowSize.Width -lt 72
  $pad = [Math]::Min(34, ($hits | ForEach-Object { $_.cmd.Length } | Measure-Object -Maximum).Maximum)
  $group = $null
  foreach ($c in $hits) {
    $g = if ($en) { $c.ge } else { $c.g }
    if ($g -ne $group) { Write-Host ''; Write-Host $g -ForegroundColor Cyan; $group = $g }
    $about = if ($en) { $c.en } else { $c.ru }
    if ($narrow) {
      Write-Host ('  ' + $c.cmd) -ForegroundColor Yellow
      Write-Host ('      ' + $about) -ForegroundColor DarkGray
    } else {
      Write-Host ('  ' + $c.cmd.PadRight($pad) + '  ') -ForegroundColor Yellow -NoNewline
      Write-Host $about
    }
  }
  if (-not $words.Count) {
    Write-Host ''
    Write-Host $(if ($en) { 'Search: cmds <word>, e.g. cmds git. On the site: the ? button.' } else { 'Поиск: cmds <слово>, например cmds git или cmds удалить. На сайте — кнопка «?».' }) -ForegroundColor DarkGray
  }
}
