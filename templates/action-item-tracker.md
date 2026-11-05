---
schema: 1
id: action-item-tracker
name: Action item tracker
version: 1.0.0
description: What was agreed, who's doing it and by when, with a box to tick off each one.
author: Thermal Studio
fields:
  - key: meeting
    label: Meeting
    placeholder: Weekly team sync
  - key: day
    label: Date
    type: date
    default: today
  - key: items
    label: Action items
    type: list
    required: true
    placeholder: What; who; when
    help: "Write each one as: what; who; when. Who and when are optional."
    default:
      - Send the slides; Ada; Friday
      - Book a room for next week; Sam
  - key: notes
    label: Notes
    type: multiline
---
# {{ meeting or "Action items" }}
{% if day %}

**{{ day | date("%A %-d %B %Y") }}**
{% endif %}

{% for item in items %}
{% set part = item | split(";", 2) | map("trim") | list %}
- [ ] **{{ part[0] }}**{% if part | length > 1 and part[1] %} · {{ part[1] }}{% endif %}{% if part | length > 2 and part[2] %} · by {{ part[2] }}{% endif +%}
{% endfor %}
{% if notes %}

{{ notes }}
{% endif %}
