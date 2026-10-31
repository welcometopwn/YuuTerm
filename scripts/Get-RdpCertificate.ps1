# Read an RDP server's TLS certificate without sending credentials.
# Performs only the X.224 negotiation (TLS/CredSSP requested) and the TLS handshake,
# then closes. Nothing is changed on the server.
param(
    [Parameter(Mandatory)][string]$HostName,
    [int]$Port = 3389,
    [int]$TimeoutMs = 5000
)
$ErrorActionPreference = 'Stop'
$client = [Net.Sockets.TcpClient]::new()
try {
    if (-not $client.ConnectAsync($HostName, $Port).Wait($TimeoutMs)) { throw "Connect to ${HostName}:$Port timed out" }
    $stream = $client.GetStream()
    $stream.ReadTimeout = $TimeoutMs
    $stream.WriteTimeout = $TimeoutMs
    # TPKT + X.224 Connection Request + RDP_NEG_REQ (PROTOCOL_SSL | PROTOCOL_HYBRID).
    [byte[]]$request = 0x03, 0x00, 0x00, 0x13, 0x0E, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00
    $stream.Write($request, 0, $request.Length)
    $header = [byte[]]::new(4)
    if ($stream.Read($header, 0, 4) -ne 4 -or $header[0] -ne 3) { throw 'Invalid TPKT response' }
    $length = $header[2] * 256 + $header[3]
    if ($length -lt 11 -or $length -gt 64) { throw "Unexpected X.224 response length $length" }
    $body = [byte[]]::new($length - 4)
    $read = 0
    while ($read -lt $body.Length) {
        $n = $stream.Read($body, $read, $body.Length - $read)
        if ($n -le 0) { throw 'Connection closed during negotiation' }
        $read += $n
    }
    if ($body.Length -lt 15) { throw 'Server offered no security negotiation (standard RDP security only)' }
    $type = $body[7]
    $selected = [BitConverter]::ToUInt32($body, 11)
    if ($type -eq 3) { throw "Server refused TLS/CredSSP negotiation (failure code $selected)" }
    if ($type -ne 2) { throw "Unexpected negotiation response type $type" }
    $script:certificate = $null
    $ssl = [Net.Security.SslStream]::new($stream, $false, {
            param($sender, $cert, $chain, $errors)
            $script:certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($cert)
            $script:policyErrors = $errors
            return $true  # Inspect only; this script never authenticates.
        })
    $ssl.AuthenticateAsClient($HostName)
    $cert = $script:certificate
    $sha256 = [Security.Cryptography.SHA256]::HashData($cert.RawData)
    [pscustomobject]@{
        Host            = "${HostName}:$Port"
        Protocol        = switch ($selected) { 1 { 'TLS' } 2 { 'CredSSP (NLA)' } 8 { 'CredSSP early auth' } default { "0x{0:X}" -f $selected } }
        Subject         = $cert.Subject
        Issuer          = $cert.Issuer
        NotBefore       = $cert.NotBefore.ToString('yyyy-MM-dd')
        NotAfter        = $cert.NotAfter.ToString('yyyy-MM-dd')
        SHA256          = ($sha256 | ForEach-Object { $_.ToString('X2') }) -join ':'
        SHA1Thumbprint  = $cert.Thumbprint
        # Candidate encodings of the server key as CredSSP sees it (lowercase hex).
        KeySha256       = ([Security.Cryptography.SHA256]::HashData($cert.GetPublicKey()) | ForEach-Object { $_.ToString('x2') }) -join ''
        SpkiSha256      = ([Security.Cryptography.SHA256]::HashData($cert.PublicKey.ExportSubjectPublicKeyInfo()) | ForEach-Object { $_.ToString('x2') }) -join ''
        WindowsTrust    = if ($script:policyErrors -eq 'None') { 'trusted' } else { [string]$script:policyErrors }
    }
    $ssl.Dispose()
}
finally {
    $client.Dispose()
}
