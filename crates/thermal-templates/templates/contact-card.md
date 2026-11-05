---
schema: 1
id: contact-card
name: Contact card
version: 1.0.0
description: Your name and details, with a QR code that saves you as a contact.
author: Thermal Studio
fields:
  - key: name
    label: Name
    required: true
    placeholder: Ada Lovelace
  - key: job
    label: Job title
  - key: org
    label: Organisation
  - key: phone
    label: Phone
    placeholder: +44 20 7946 0000
  - key: email
    label: Email
  - key: website
    label: Website
  - key: show_details
    label: Print the details under the code
    type: bool
    default: true
---
# {{ name }}
{% if job or org %}

{{ job }}{% if job and org %}, {% endif %}{{ org }}
{% endif %}

```qr
BEGIN:VCARD
VERSION:3.0
N:;{{ name | vcard_escape }};;;
FN:{{ name | vcard_escape }}
{% if org %}
ORG:{{ org | vcard_escape }}
{% endif %}
{% if job %}
TITLE:{{ job | vcard_escape }}
{% endif %}
{% if phone %}
TEL;TYPE=CELL:{{ phone | vcard_escape }}
{% endif %}
{% if email %}
EMAIL:{{ email | vcard_escape }}
{% endif %}
{% if website %}
URL:{{ website | vcard_escape }}
{% endif %}
END:VCARD
```

*Scan to save this contact.*
{% if show_details %}
{% if phone %}

**Phone:** {{ phone }}
{% endif %}
{% if email %}

**Email:** {{ email }}
{% endif %}
{% if website %}

**Web:** {{ website }}
{% endif %}
{% endif %}
