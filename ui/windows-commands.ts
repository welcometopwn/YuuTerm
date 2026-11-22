import type { Catalog, SshCommand } from "./ssh-commands";

const ps = (value?: string) => `'${value ?? ""}'`;
const table = "Format-Table -AutoSize";

const COMMANDS: SshCommand[] = [
  {
    id: "win-system-info",
    category: "System",
    label: "System information",
    build: () => "systeminfo",
  },
  {
    id: "win-disks",
    category: "System",
    label: "Disk space",
    build: () =>
      `Get-PSDrive -PSProvider FileSystem | Select-Object Name, @{n='Used GB';e={[math]::Round($_.Used/1GB,1)}}, @{n='Free GB';e={[math]::Round($_.Free/1GB,1)}}, Root | ${table}`,
  },
  {
    id: "win-services",
    category: "Services",
    label: "All services",
    build: () =>
      `Get-Service | Sort-Object Status, DisplayName | ${table} Status, Name, DisplayName`,
  },
  {
    id: "win-services-running",
    category: "Services",
    label: "Running services",
    build: () =>
      `Get-Service | Where-Object Status -eq 'Running' | ${table} Name, DisplayName`,
  },
  {
    id: "win-service-status",
    category: "Services",
    label: "Service status",
    param: "service",
    build: (c) => `Get-Service -Name ${ps(c.value)} | Format-List *`,
  },
  {
    id: "win-service-control",
    category: "Services",
    label: "Restart, stop or start a service",
    param: "service",
    danger: true,
    actions: [
      { value: "Restart", label: "Restart" },
      { value: "Stop", label: "Stop" },
      { value: "Start", label: "Start" },
    ],
    build: (c) =>
      `${c.action ?? "Restart"}-Service -Name ${ps(c.value)}${c.action === "Start" ? "" : " -Force"}`,
  },
  {
    id: "win-top-cpu",
    category: "Processes",
    label: "Top processes by CPU time",
    build: () =>
      `Get-Process | Sort-Object CPU -Descending | Select-Object -First 20 Id, ProcessName, @{n='CPU s';e={[int]$_.CPU}}, @{n='MB';e={[int]($_.WorkingSet64/1MB)}} | ${table}`,
  },
  {
    id: "win-top-memory",
    category: "Processes",
    label: "Top processes by memory",
    build: () =>
      `Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 20 Id, ProcessName, @{n='MB';e={[int]($_.WorkingSet64/1MB)}} | ${table}`,
  },
  {
    id: "win-kill",
    category: "Processes",
    label: "End process",
    param: "pid",
    danger: true,
    build: (c) => `Stop-Process -Id ${c.value ?? ""} -Force`,
  },
  {
    id: "win-events-system",
    category: "Event log",
    label: "Recent system errors",
    build: () =>
      `Get-WinEvent -FilterHashtable @{LogName='System'; Level=1,2} -MaxEvents 20 | ${table} -Wrap TimeCreated, Id, ProviderName, Message`,
  },
  {
    id: "win-events-application",
    category: "Event log",
    label: "Recent application errors",
    build: () =>
      `Get-WinEvent -FilterHashtable @{LogName='Application'; Level=1,2} -MaxEvents 20 | ${table} -Wrap TimeCreated, Id, ProviderName, Message`,
  },
  {
    id: "win-events-boot",
    category: "Event log",
    label: "Starts, shutdowns and crashes",
    build: () =>
      `Get-WinEvent -FilterHashtable @{LogName='System'; Id=41,1074,6005,6006,6008} -MaxEvents 20 | ${table} -Wrap TimeCreated, Id, Message`,
  },
  {
    id: "win-users",
    category: "Sessions",
    label: "Signed-in users",
    build: () => "query user",
  },
  {
    id: "win-logoff",
    category: "Sessions",
    label: "Sign out a session",
    param: "session",
    danger: true,
    build: (c) => `logoff ${c.value ?? ""}`,
  },
  {
    id: "win-ipconfig",
    category: "Network",
    label: "IP configuration",
    build: () => "ipconfig /all",
  },
  {
    id: "win-adapters",
    category: "Network",
    label: "Network adapters",
    build: () =>
      `Get-NetAdapter | ${table} Name, Status, LinkSpeed, MacAddress, InterfaceDescription`,
  },
  {
    id: "win-listening",
    category: "Network",
    label: "Listening ports",
    build: () =>
      `Get-NetTCPConnection -State Listen | Sort-Object LocalPort | Select-Object LocalAddress, LocalPort, OwningProcess, @{n='Process';e={(Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue).ProcessName}} | ${table}`,
  },
  {
    id: "win-ping",
    category: "Network",
    label: "Ping",
    param: "host",
    build: (c) => `Test-Connection -ComputerName ${ps(c.value)} -Count 4`,
  },
  {
    id: "win-hotfixes",
    category: "Windows Update",
    label: "Installed updates",
    build: () =>
      `Get-HotFix | Sort-Object InstalledOn -Descending | Select-Object -First 20 HotFixID, Description, InstalledOn | ${table}`,
  },
  {
    id: "win-update-search",
    category: "Windows Update",
    label: "Search for updates",
    build: () =>
      `(New-Object -ComObject Microsoft.Update.Session).CreateUpdateSearcher().Search('IsInstalled=0 and IsHidden=0').Updates | Select-Object Title | ${table}`,
  },
  {
    id: "win-update-scan",
    category: "Windows Update",
    label: "Scan, download and install updates",
    danger: true,
    build: () => "UsoClient StartInteractiveScan",
  },
];

export const WINDOWS_CATALOG: Catalog = {
  commands: COMMANDS,
  categories: [...new Set(COMMANDS.map((c) => c.category))],
  description:
    "Commands for the RDP remote tools, typed into Windows PowerShell on the remote computer. Custom commands are typed exactly as entered. Drag the grip to reorder.",
};
