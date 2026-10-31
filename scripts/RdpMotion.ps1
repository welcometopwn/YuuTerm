# Runs ON the RDP test host inside the logged-on desktop (interactive scheduled
# task started by Measure-RdpIdle.ps1 -Motion): a maximized console that scrolls
# full lines of text for a fixed time, a repeatable full-screen motion load.
param([int]$Seconds = 50)
Add-Type -Name Win -Namespace Native -MemberDefinition '[DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr h, int c); [DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow();'
[void][Native.Win]::ShowWindow([Native.Win]::GetConsoleWindow(), 3)
$line = ('YuuTerm RDP motion ' * 12)
$end = (Get-Date).AddSeconds($Seconds)
$n = 0
while ((Get-Date) -lt $end) {
    $n++
    [Console]::Out.WriteLine("$n $line")
}
