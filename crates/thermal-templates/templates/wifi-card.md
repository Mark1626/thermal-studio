---
schema: 1
id: wifi-card
name: Wi-Fi card
version: 1.0.0
description: Your network name and password, with a QR code guests can scan to join.
author: Thermal Studio
fields:
  - key: ssid
    label: Network name
    required: true
    placeholder: MyHomeWiFi
  - key: password
    label: Password
  - key: security
    label: Security
    type: choice
    options:
      - { value: WPA, label: WPA/WPA2/WPA3 }
      - { value: WEP, label: WEP }
      - { value: nopass, label: None (open network) }
  - key: hidden
    label: Hidden network
    type: bool
    help: Turn on if the network doesn't appear in the list of networks.
  - key: show_password
    label: Print the password
    type: bool
    default: true
    help: For devices that can't scan the code.
---
# Wi-Fi

```qr
WIFI:T:{{ security }};S:{{ ssid | wifi_escape }};{% if security != "nopass" %}P:{{ password | wifi_escape }};{% endif %}{% if hidden %}H:true;{% endif %};
```

*Point your phone's camera at the code to join.*

**Network:** {{ ssid }}
{% if show_password and security != "nopass" %}

**Password:** {{ password }}
{% endif %}
