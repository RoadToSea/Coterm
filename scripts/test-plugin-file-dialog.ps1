param(
    [Parameter(Mandatory = $true)][int]$TaskProcessId,
    [string]$PackagePath,
    [switch]$Cancel
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$scope = [System.Windows.Automation.TreeScope]
$element = [System.Windows.Automation.AutomationElement]
$condition = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition($element::ProcessIdProperty, $TaskProcessId)),
    (New-Object System.Windows.Automation.PropertyCondition($element::NameProperty, 'Choose offline plugin package'))
)
$deadline = [DateTime]::UtcNow.AddSeconds(15)
$dialog = $null
while ([DateTime]::UtcNow -lt $deadline) {
    $dialog = $element::RootElement.FindFirst($scope::Children, $condition)
    if ($dialog) { break }
    Start-Sleep -Milliseconds 100
}
if (-not $dialog) { throw 'Native offline package dialog did not appear.' }
if ($Cancel) {
    $window = $dialog.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern)
    $window.Close()
    exit 0
}
if (-not (Test-Path -LiteralPath $PackagePath -PathType Leaf)) { throw 'Test package is missing.' }
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class CotermFileDialog {
    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageW")]
    public static extern IntPtr SetText(IntPtr window, uint message, IntPtr parameter, string text);
    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr parameter, IntPtr source);
}
'@
# Common-file-dialog Win32 controls can appear as UIA panes without Value/Invoke patterns.
$filename = $dialog.FindAll($scope::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition($element::AutomationIdProperty, '1148'))) |
    Where-Object { $_.Current.ClassName -eq 'Edit' } | Select-Object -First 1
if (-not $filename) { throw 'Native file name input is missing.' }
[CotermFileDialog]::SetText([IntPtr]$filename.Current.NativeWindowHandle, 0x000C, [IntPtr]::Zero, [IO.Path]::GetFullPath($PackagePath)) | Out-Null
$open = $dialog.FindFirst($scope::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition($element::AutomationIdProperty, '1')))
if (-not $open) { throw 'Native Open button is missing.' }
if (-not [CotermFileDialog]::PostMessage([IntPtr]$dialog.Current.NativeWindowHandle, 0x0111, [IntPtr]1, [IntPtr]$open.Current.NativeWindowHandle)) {
    throw 'Could not activate the native Open button.'
}
