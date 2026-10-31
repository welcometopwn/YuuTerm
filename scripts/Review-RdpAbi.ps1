param([string]$OutputPath = 'docs/evidence/security-review-rdp-abi.json')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$output = [IO.Path]::GetFullPath((Join-Path $root $OutputPath))
if (-not $output.StartsWith(($root + [IO.Path]::DirectorySeparatorChar), [StringComparison]::OrdinalIgnoreCase)) { throw 'Output must remain inside the clone' }
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using TYPEDESC = System.Runtime.InteropServices.ComTypes.TYPEDESC;
using TYPEATTR = System.Runtime.InteropServices.ComTypes.TYPEATTR;
using TYPELIBATTR = System.Runtime.InteropServices.ComTypes.TYPELIBATTR;
using FUNCDESC = System.Runtime.InteropServices.ComTypes.FUNCDESC;
using ELEMDESC = System.Runtime.InteropServices.ComTypes.ELEMDESC;
using SYSKIND = System.Runtime.InteropServices.ComTypes.SYSKIND;
public static class RdpAbiReview {
    [DllImport("oleaut32.dll", CharSet=CharSet.Unicode, PreserveSig=false)]
    static extern void LoadTypeLibEx(string path, int kind, out ITypeLib library);
    static string TypeName(ITypeInfo info, TYPEDESC type) {
        var kind = (VarEnum)type.vt;
        if (kind == VarEnum.VT_PTR || kind == VarEnum.VT_SAFEARRAY)
            return kind + "<" + TypeName(info, (TYPEDESC)Marshal.PtrToStructure(type.lpValue, typeof(TYPEDESC))) + ">";
        if (kind == VarEnum.VT_USERDEFINED) {
            ITypeInfo reference; info.GetRefTypeInfo(type.lpValue.ToInt32(), out reference);
            try { string name, text, help; int context; reference.GetDocumentation(-1, out name, out text, out context, out help); return name; }
            finally { Marshal.ReleaseComObject(reference); }
        }
        return kind.ToString();
    }
    public static object[] Inspect(string path) {
        ITypeLib library; LoadTypeLibEx(path, 2, out library); // REGKIND_NONE: no registration
        var rows = new List<object>();
        try {
            IntPtr libPointer; library.GetLibAttr(out libPointer);
            var attr = (TYPELIBATTR)Marshal.PtrToStructure(libPointer, typeof(TYPELIBATTR));
            library.ReleaseTLibAttr(libPointer);
            int pointerSize = attr.syskind == SYSKIND.SYS_WIN64 ? 8 : 4;
            foreach (string guid in new [] {"C1E6743A-41C1-4A74-832A-0DD06C1C7A0E", "4F6996D5-D7B1-412C-B0FF-063718566907", "2F079C4C-87B2-4AFD-97AB-20CDB43038AE"}) {
                var id = new Guid(guid); ITypeInfo first; library.GetTypeInfoOfGuid(ref id, out first);
                var chain = new Queue<ITypeInfo>(); chain.Enqueue(first);
                var seen = new HashSet<Guid>();
                while (chain.Count != 0) {
                var info = chain.Dequeue();
                try {
                    IntPtr attrPointer; info.GetTypeAttr(out attrPointer);
                    var type = (TYPEATTR)Marshal.PtrToStructure(attrPointer, typeof(TYPEATTR));
                    info.ReleaseTypeAttr(attrPointer);
                    if (!seen.Add(type.guid)) continue;
                    for (int baseIndex=0; baseIndex<type.cImplTypes; baseIndex++) {
                        int reference; info.GetRefTypeOfImplType(baseIndex, out reference);
                        ITypeInfo parent; info.GetRefTypeInfo(reference, out parent); chain.Enqueue(parent);
                    }
                    for (int i=0; i<type.cFuncs; i++) {
                        IntPtr functionPointer; info.GetFuncDesc(i, out functionPointer);
                        try {
                            var function = (FUNCDESC)Marshal.PtrToStructure(functionPointer, typeof(FUNCDESC));
                            string name, text, help; int context; info.GetDocumentation(function.memid, out name, out text, out context, out help);
                            if (name != "ClearTextPassword" && name != "UIParentWindowHandle" && name != "AllowCredentialSaving" && name != "AllowPromptingForCredentials" && name != "SendKeys") continue;
                            var parameters = new List<string>();
                            for (int p=0; p<function.cParams; p++) {
                                var element = (ELEMDESC)Marshal.PtrToStructure(IntPtr.Add(function.lprgelemdescParam, p * Marshal.SizeOf(typeof(ELEMDESC))), typeof(ELEMDESC));
                                parameters.Add(TypeName(info, element.tdesc));
                            }
                            rows.Add(new {iid=guid, name=name, invoke=function.invkind.ToString(), slot=function.oVft/pointerSize, returns=TypeName(info,function.elemdescFunc.tdesc), parameters=parameters.ToArray()});
                        } finally { info.ReleaseFuncDesc(functionPointer); }
                    }
                } finally { Marshal.ReleaseComObject(info); }
                }
            }
        } finally { Marshal.ReleaseComObject(library); }
        return rows.ToArray();
    }
}
'@
$dll = Join-Path $env:SystemRoot 'System32/mstscax.dll'
$result = [ordered]@{source='Windows System32/mstscax.dll type library, REGKIND_NONE';version=(Get-Item -LiteralPath $dll).VersionInfo.FileVersion;methods=[RdpAbiReview]::Inspect($dll)}
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $output -Encoding utf8
Write-Output "Recorded $($result.methods.Count) ABI methods in $OutputPath"
