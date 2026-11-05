---
schema: 1
id: checklist
name: Checklist
version: 1.0.0
description: A list with tick boxes, for shopping, packing or chores.
author: Thermal Studio
fields:
  - key: title
    label: Title
    default: Shopping list
  - key: items
    label: Items
    type: list
    placeholder: Add an item
    default: [Milk, Bread, Eggs]
  - key: notes
    label: Notes
    type: multiline
    help: Printed under the list.
---
# {{ title }}

{% for item in items %}
- [ ] {{ item }}
{% endfor %}
{% if notes %}

{{ notes }}
{% endif %}
