# Thermal notes

Printed by **oss-thermal** on a *58 mm* roll. Supports `inline code`, ~~strikethrough~~ and [links](https://example.com).

## Today

- [x] Reverse-engineer the BLE protocol
- [ ] Write the Rust driver
  - dithering
  - markdown
- Buy paper

வணக்கம்! தமிழிலும் அச்சிடலாம்.

1. Wake the printer
2. Send `10 FF F1 03`
3. Stream the raster

> Isolated single dots smear on thermal paper, so prefer clustered patterns for shading.

### Receipt

| Item       | Qty |  Price |
|:-----------|:---:|-------:|
| Coffee     |  2  |  ₹240 |
| Croissant  |  1  |  ₹180 |
| **Total**  |     | **₹420** |

```
GS v 0  1D 76 30 00 30 00 hL hH
```

---

![test chart](test-chart.png)

That's all, folks.
