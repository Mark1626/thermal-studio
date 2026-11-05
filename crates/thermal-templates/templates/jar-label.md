---
schema: 1
id: jar-label
name: Jar label
version: 1.0.0
description: What's inside, when it was made and when to use it by.
author: Thermal Studio
fields:
  - key: contents
    label: Contents
    required: true
    placeholder: Strawberry jam
  - key: made
    label: Made on
    type: date
    default: today
  - key: use_within
    label: Use within
    type: choice
    options:
      - { value: "7", label: 1 week }
      - { value: "14", label: 2 weeks }
      - { value: "30", label: 1 month }
      - { value: "90", label: 3 months }
      - { value: "180", label: 6 months }
      - { value: "365", label: 1 year }
      - { value: "none", label: "Don't print a use-by date" }
    default: "30"
  - key: note
    label: Note
    placeholder: Keep in the fridge once opened
---
# {{ contents }}

{% if made %}
Made **{{ made | date }}**
{% endif %}
{% if made and use_within != "none" %}

Use by **{{ made | add_days(use_within | int) | date }}**
{% endif %}
{% if note %}

*{{ note }}*
{% endif %}
