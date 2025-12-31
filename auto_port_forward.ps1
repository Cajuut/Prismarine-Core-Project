param (
    [int]$Port = 25565,
    [string]$Protocol = "TCP",
    [string]$Description = "FastServer_Auto",
    [string]$ManualIP = $null,
    [switch]$VerboseLog,
    [switch]$Close
)

function Write-Log {
    param([string]$Message, [ConsoleColor]$Color = "White")
    Write-Host "[$(Get-Date -Format 'HH:mm:ss')] $Message" -ForegroundColor $Color
}

function Get-LocalIP {
    try {
        # Prioritize interfaces that have a Default Gateway (internet connected)
        $ip = (Get-NetIPAddress -AddressFamily IPv4 -InterfaceAlias *Wi-Fi*, *Ethernet* | Where-Object { 
                $_.IPAddress -like "192.168.*" -and $_.InterfaceIndex -in (Get-NetRoute -DestinationPrefix "0.0.0.0/0").InterfaceIndex 
            } | Select-Object -First 1).IPAddress
        
        if ([string]::IsNullOrEmpty($ip)) {
            # Fallback to simple filtering
            $ip = (Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.IPAddress -like "192.168.*" -and $_.PrefixOrigin -ne "WellKnown" } | Select-Object -First 1).IPAddress
        }
        return $ip
    }
    catch {
        return $null
    }
}

function Find-UpnpRouter {
    Write-Log "Searching for UPnP Router (SSDP)..." "Cyan"
    $ssdpMsg = "M-SEARCH * HTTP/1.1`r`n" +
    "HOST: 239.255.255.250:1900`r`n" +
    "MAN: `"ssdp:discover`"`r`n" +
    "MX: 3`r`n" +
    "ST: urn:schemas-upnp-org:service:WANPPPConnection:1`r`n" +
    "`r`n"
    
    $bytes = [System.Text.Encoding]::ASCII.GetBytes($ssdpMsg)
    $udp = New-Object System.Net.Sockets.UdpClient
    $udp.Client.ReceiveTimeout = 3000 # 3 seconds timeout
    
    try {
        $udp.Connect("239.255.255.250", 1900)
        $udp.Send($bytes, $bytes.Length) | Out-Null
        
        $remoteEp = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
        
        # Listen for response (try a few times)
        for ($i = 0; $i -lt 3; $i++) {
            try {
                $recvBytes = $udp.Receive([ref]$remoteEp)
                $response = [System.Text.Encoding]::ASCII.GetString($recvBytes)
                
                if ($response -match "Location: (http://.+?)\s") {
                    $location = $matches[1]
                    Write-Log "Router found at: $location" "Green"
                    return $location
                }
            }
            catch {
                # Timeout, ignore
            }
        }
    }
    finally {
        $udp.Close()
    }
    return $null
}

function Get-ControlURL {
    param([string]$DescURL)
    
    try {
        $xml = Invoke-WebRequest -Uri $DescURL -UseBasicParsing
        [xml]$doc = $xml.Content
        
        # Namespace manager for parsing
        $ns = New-Object System.Xml.XmlNamespaceManager($doc.NameTable)
        $ns.AddNamespace("n", "urn:schemas-upnp-org:device-1-0")
        
        # Find WANPPPConnection service
        $services = $doc.GetElementsByTagName("service")
        foreach ($svc in $services) {
            if ($svc.serviceType -like "*WANPPPConnection:1*") {
                $ctrlPath = $svc.controlURL
                
                # Handle relative URL
                if (-not $ctrlPath.StartsWith("http")) {
                    $uri = New-Object System.Uri($DescURL)
                    if ($ctrlPath.StartsWith("/")) {
                        return "http://" + $uri.Host + ":" + $uri.Port + $ctrlPath
                    }
                    else {
                        return "http://" + $uri.Host + ":" + $uri.Port + "/upnp/control/" + $ctrlPath # Approximate fallback
                    }
                }
                return $ctrlPath
            }
        }
    }
    catch {
        Write-Log "Failed to parse router description: $_" "Red"
    }
    return $null
}

# --- Main Execution ---

try {
    $mode = if ($Close) { "CLOSING" } else { "OPENING" }
    Write-Log "--- Auto Port Forwarding Tool [$mode] ---" "Cyan"
    
    # 1. Local IP
    $localIP = $ManualIP
    if ([string]::IsNullOrEmpty($localIP)) {
        $localIP = Get-LocalIP
    }
    
    if ([string]::IsNullOrEmpty($localIP)) {
        throw "Could not determine Local IP. Please specify with -ManualIP."
    }
    Write-Log "Local IP: $localIP"
    
    # 2. Router Discovery
    $descUrl = Find-UpnpRouter
    
    # Fallback if discovery fails (Manual override for known NEC/Typical routers)
    if ([string]::IsNullOrEmpty($descUrl)) {
        Write-Log "SSDP Discovery failed or timed out." "Yellow"
        Write-Log "Trying default NEC router URL..." "Yellow"
        $controlUrl = "http://192.168.0.1:2869/upnp/control/WANPPPConn1" 
    }
    else {
        $controlUrl = Get-ControlURL -DescURL $descUrl
    }
    
    if ([string]::IsNullOrEmpty($controlUrl)) {
        throw "Could not determine Router Control URL."
    }
    Write-Log "Target Control URL: $controlUrl"

    # 3. Port Action
    if ($Close) {
        # DELETE MAPPING
        Write-Log "Attempting to DELETE port mapping..."
        $action = '"urn:schemas-upnp-org:service:WANPPPConnection:1#DeletePortMapping"'
        $body = @"
<?xml version="1.0"?>
<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/" SOAP-ENV:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
<SOAP-ENV:Body>
    <m:DeletePortMapping xmlns:m="urn:schemas-upnp-org:service:WANPPPConnection:1">
        <NewRemoteHost></NewRemoteHost>
        <NewExternalPort>$Port</NewExternalPort>
        <NewProtocol>$Protocol</NewProtocol>
    </m:DeletePortMapping>
</SOAP-ENV:Body>
</SOAP-ENV:Envelope>
"@
    }
    else {
        # ADD MAPPING
        Write-Log "Attempting to ADD port mapping..."
        $action = '"urn:schemas-upnp-org:service:WANPPPConnection:1#AddPortMapping"'
        $body = @"
<?xml version="1.0"?>
<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/" SOAP-ENV:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
<SOAP-ENV:Body>
    <m:AddPortMapping xmlns:m="urn:schemas-upnp-org:service:WANPPPConnection:1">
        <NewRemoteHost></NewRemoteHost>
        <NewInternalClient>$localIP</NewInternalClient>
        <NewProtocol>$Protocol</NewProtocol>
        <NewExternalPort>$Port</NewExternalPort>
        <NewInternalPort>$Port</NewInternalPort>
        <NewEnabled>1</NewEnabled>
        <NewLeaseDuration>0</NewLeaseDuration>
        <NewPortMappingDescription>$Description</NewPortMappingDescription>
    </m:AddPortMapping>
</SOAP-ENV:Body>
</SOAP-ENV:Envelope>
"@
    }

    $response = Invoke-WebRequest -Uri $controlUrl -Method Post -Body $body -ContentType "text/xml" -Headers @{ "SOAPAction" = $action } -UserAgent "Java/17.0.12" -UseBasicParsing
    
    if ($response.StatusCode -eq 200) {
        if ($Close) {
            Write-Log "`n[SUCCESS] Port $Port ($Protocol) mapping DELETED." "Green"
        }
        else {
            Write-Log "`n[SUCCESS] Port $Port ($Protocol) forwarded to $localIP" "Green"
        }
    }
    else {
        throw "HTTP Status: $($response.StatusCode)"
    }

}
catch {
    Write-Log "`n[ERROR] Operation failed." "Red"
    Write-Log "Message: $_" "Red"
    
    if ($_.Exception.Response) {
        try {
            $reader = New-Object System.IO.StreamReader $_.Exception.Response.GetResponseStream()
            $errContent = $reader.ReadToEnd()
            Write-Log "Router Response: $errContent" "Yellow"
            
            if ($errContent -match "<errorDescription>(.+?)</errorDescription>") {
                Write-Log "Reason: $($matches[1])" "Red"
            }
        }
        catch {}
    }
}
finally {
    Write-Log "`nDone." "Gray"
}
