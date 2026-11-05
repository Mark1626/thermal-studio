---
schema: 1
id: calendar-tracker
name: Calendar tracker
version: 1.0.0
description: A month of days to tick off, for a habit, a medicine or practice.
author: Thermal Studio
fields:
  - key: title
    label: What you're tracking
    required: true
    placeholder: Morning walk
  - key: month
    label: Month
    type: date
    default: today
    help: Any day in the month to print.
  - key: week_starts
    label: Weeks start on
    type: choice
    options: [Monday, Sunday]
  - key: goal
    label: Goal
    placeholder: 20 days
---
{% set first = month | add_days(1 - (month | date("%-d") | int)) %}
{% set this_month = first | date("%m") %}
{% set weekday = first | date("%u") | int %}
{% set lead = weekday % 7 if week_starts == "Sunday" else weekday - 1 %}
{% set start = first | add_days(0 - lead) %}
# {{ title }}

**{{ first | date("%B %Y") }}**{% if goal %} · Goal: {{ goal }}{% endif +%}

{# Spaces that don't collapse widen the columns, so there's room to tick each day. #}
{% if week_starts == "Sunday" %}
| &nbsp;Su&nbsp; | &nbsp;Mo&nbsp; | &nbsp;Tu&nbsp; | &nbsp;We&nbsp; | &nbsp;Th&nbsp; | &nbsp;Fr&nbsp; | &nbsp;Sa&nbsp; |
{% else %}
| &nbsp;Mo&nbsp; | &nbsp;Tu&nbsp; | &nbsp;We&nbsp; | &nbsp;Th&nbsp; | &nbsp;Fr&nbsp; | &nbsp;Sa&nbsp; | &nbsp;Su&nbsp; |
{% endif %}
|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
{% for week in range(6) %}
{% set week_start = start | add_days(week * 7) %}
{% if week == 0 or (week_start | date("%m")) == this_month %}
|{% for d in range(7) %}{% set day = week_start | add_days(d) %} {% if (day | date("%m")) == this_month %}{{ day | date("%-d") }}{% endif %} |{% endfor +%}
{% endif %}
{% endfor %}
