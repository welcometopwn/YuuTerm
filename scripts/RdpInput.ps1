# Shared by the RDP test scripts: synthetic keyboard/mouse input and the
# physical rectangle of the RDP host window.
if (-not ('Drive' -as [type])) {
Add-Type @"
using System; using System.Text; using System.Threading; using System.Runtime.InteropServices;
public static class Drive {
  [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk; public ushort scan; public uint flags; public uint time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint data; public uint flags; public uint time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public KEYBDINPUT k; [FieldOffset(0)] public MOUSEINPUT m; }
  [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int size);
  [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  public struct RECT { public int L, T, R, B; }
  static void Key(ushort vk, bool up) { var i = new INPUT { type = 1 }; i.u.k.vk = vk; i.u.k.flags = up ? 2u : 0u; SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT))); Thread.Sleep(30); }
  public static void Combo(params ushort[] vks) { foreach (var v in vks) Key(v, false); for (int i = vks.Length - 1; i >= 0; i--) Key(vks[i], true); Thread.Sleep(150); }
  public static void Text(string s) {
    foreach (var c in s) {
      var d = new INPUT { type = 1 }; d.u.k.scan = c; d.u.k.flags = 4;
      var u = new INPUT { type = 1 }; u.u.k.scan = c; u.u.k.flags = 4 | 2;
      SendInput(2, new[] { d, u }, Marshal.SizeOf(typeof(INPUT))); Thread.Sleep(8);
    }
  }
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern short VkKeyScan(char c);
  // Types with virtual-key events (the RDP client forwards these; it ignores
  // injected KEYEVENTF_UNICODE input). Only characters on the local layout.
  public static void Keys(string s) {
    foreach (var c in s) {
      short code = VkKeyScan(c);
      if (code == -1) throw new ArgumentException("Character not on the keyboard layout: " + c);
      ushort vk = (ushort)(code & 0xff); int mods = (code >> 8) & 0xff;
      if ((mods & 1) != 0) Key(0x10, false);
      if ((mods & 2) != 0) Key(0x11, false);
      if ((mods & 4) != 0) Key(0x12, false);
      Key(vk, false); Key(vk, true);
      if ((mods & 4) != 0) Key(0x12, true);
      if ((mods & 2) != 0) Key(0x11, true);
      if ((mods & 1) != 0) Key(0x10, true);
    }
  }
  public static void RightClick(int x, int y) {
    SetCursorPos(x, y); Thread.Sleep(100);
    var d = new INPUT { type = 0 }; d.u.m.flags = 8; var u = new INPUT { type = 0 }; u.u.m.flags = 16;
    SendInput(2, new[] { d, u }, Marshal.SizeOf(typeof(INPUT))); Thread.Sleep(300);
  }
  public static void Click(int x, int y) {
    SetCursorPos(x, y); Thread.Sleep(100);
    var d = new INPUT { type = 0 }; d.u.m.flags = 2; var u = new INPUT { type = 0 }; u.u.m.flags = 4;
    SendInput(2, new[] { d, u }, Marshal.SizeOf(typeof(INPUT))); Thread.Sleep(200);
  }
  // Physical-pixel rectangle of the RDP host child (class AtlAxWin) of a top-level window.
  public static int[] RdpRect(IntPtr top) {
    for (var c = GetWindow(top, 5); c != IntPtr.Zero; c = GetWindow(c, 2)) {
      var s = new StringBuilder(64); GetClassName(c, s, 64);
      if (s.ToString() == "AtlAxWin") { RECT r; GetWindowRect(c, out r); return new[] { r.L, r.T, r.R, r.B }; }
    }
    return null;
  }
}
"@
}
