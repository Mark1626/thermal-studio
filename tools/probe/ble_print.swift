import Foundation
import CoreBluetooth
import CoreGraphics
import CoreText
import ImageIO

// Single test print over BLE using the LuckPrinter "Lujiang" sequence:
//   00 x12 (wake) -> 10 FF F1 03 (enable) -> GS v 0 raster -> 1B 4A n (feed) -> 10 FF F1 45 (end)
// Usage:
//   ble_print <preview.png>            render only, write a 1-bpp preview PNG
//   ble_print <preview.png> --print    render, check status, then print once

let W = 384, H = 200, BPR = W / 8   // 48 bytes per row
let args = CommandLine.arguments
let previewPath = args.count > 1 ? args[1] : "preview.png"
let doPrint = args.contains("--print")

// MARK: - Render

func renderBits() -> [UInt8] {
    let ctx = CGContext(data: nil, width: W, height: H, bitsPerComponent: 8, bytesPerRow: W,
                        space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)!
    ctx.setFillColor(gray: 1, alpha: 1); ctx.fill(CGRect(x: 0, y: 0, width: W, height: H))
    ctx.setFillColor(gray: 0, alpha: 1); ctx.setStrokeColor(gray: 0, alpha: 1)
    ctx.textMatrix = .identity

    // top-left based rect helper
    func r(_ x: Int, _ y: Int, _ w: Int, _ h: Int) -> CGRect { CGRect(x: x, y: H - y - h, width: w, height: h) }
    func text(_ s: String, size: CGFloat, bold: Bool, baseline: CGFloat) {
        let font = CTFontCreateWithName((bold ? "Menlo-Bold" : "Menlo-Regular") as CFString, size, nil)
        let attrs = [kCTFontAttributeName as NSAttributedString.Key: font,
                     kCTForegroundColorAttributeName as NSAttributedString.Key: CGColor(gray: 0, alpha: 1)] as [NSAttributedString.Key: Any]
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: s, attributes: attrs))
        let w = CTLineGetTypographicBounds(line, nil, nil, nil)
        ctx.textPosition = CGPoint(x: (CGFloat(W) - CGFloat(w)) / 2, y: CGFloat(H) - baseline)
        CTLineDraw(line, ctx)
    }

    // 2 px border touching all four edges (detects left/right cropping)
    ctx.fill(r(0, 0, W, 2)); ctx.fill(r(0, H - 2, W, 2)); ctx.fill(r(0, 0, 2, H)); ctx.fill(r(W - 2, 0, 2, H))

    let df = DateFormatter(); df.dateFormat = "yyyy-MM-dd HH:mm:ss"
    text("OSS-THERMAL TEST", size: 26, bold: true, baseline: 34)
    text("Seznik MiniX  S1_EY628  V1.6.23", size: 15, bold: false, baseline: 58)
    text("BLE FF00/FF02 | Lujiang | GS v 0", size: 15, bold: false, baseline: 78)
    text(df.string(from: Date()), size: 15, bold: true, baseline: 98)

    // ruler: tick every 8 px, long tick every 48 px, across the full 384 px
    for x in stride(from: 0, to: W, by: 8) {
        let h = (x % 48 == 0) ? 16 : 8
        ctx.fill(r(x, 108, 1, h))
    }
    ctx.fill(r(W - 1, 108, 1, 16))   // right-most pixel column
    ctx.fill(r(0, 124, W, 1))        // ruler baseline

    // grab gray buffer (memory row 0 = top of image)
    let gray = ctx.data!.bindMemory(to: UInt8.self, capacity: W * H)
    var px = [Bool](repeating: false, count: W * H)
    for i in 0..<(W * H) { px[i] = gray[i] < 128 }

    // density swatches drawn directly in 1-bpp space: solid, 50 % checker, 25 % dots, 2-px lines
    let swY = 134, swH = 54, swW = 80, gap = 16
    let x0 = (W - (4 * swW + 3 * gap)) / 2
    for s in 0..<4 {
        let sx = x0 + s * (swW + gap)
        for y in swY..<(swY + swH) {
            for x in sx..<(sx + swW) {
                let on: Bool
                switch s {
                case 0: on = true
                case 1: on = (x + y) % 2 == 0
                case 2: on = x % 2 == 0 && y % 2 == 0
                default: on = (y / 2) % 2 == 0
                }
                if on { px[y * W + x] = true }
            }
        }
    }

    // pack: MSB first, 1 = black
    var bits = [UInt8](repeating: 0, count: BPR * H)
    for y in 0..<H { for x in 0..<W where px[y * W + x] { bits[y * BPR + x / 8] |= UInt8(0x80 >> (x % 8)) } }
    return bits
}

func writePreview(_ bits: [UInt8], to path: String) {
    let scale = 2
    var buf = [UInt8](repeating: 255, count: W * scale * H * scale)
    for y in 0..<(H * scale) {
        for x in 0..<(W * scale) {
            let sx = x / scale, sy = y / scale
            if bits[sy * BPR + sx / 8] & UInt8(0x80 >> (sx % 8)) != 0 { buf[y * W * scale + x] = 0 }
        }
    }
    let provider = CGDataProvider(data: Data(buf) as CFData)!
    let img = CGImage(width: W * scale, height: H * scale, bitsPerComponent: 8, bitsPerPixel: 8, bytesPerRow: W * scale,
                      space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGBitmapInfo(rawValue: 0),
                      provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: path) as CFURL, "public.png" as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, img, nil); CGImageDestinationFinalize(dest)
}

let bits = renderBits()
writePreview(bits, to: previewPath)
let inked = bits.reduce(0) { $0 + $1.nonzeroBitCount }
print("rendered \(W)x\(H), \(bits.count) bytes, \(inked) black dots (\(String(format: "%.1f", Double(inked) * 100 / Double(W * H)))%) -> \(previewPath)")
if !doPrint { exit(0) }

// MARK: - BLE

let svc = CBUUID(string: "FF00"), cWrite = CBUUID(string: "FF02"), cN1 = CBUUID(string: "FF01"), cN3 = CBUUID(string: "FF03")
let t0 = Date()
func ts() -> String { String(format: "%7.3fs", Date().timeIntervalSince(t0)) }
func hex(_ d: Data) -> String { d.map { String(format: "%02X", $0) }.joined(separator: " ") }

final class Printer: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var cm: CBCentralManager!
    var p: CBPeripheral?
    var wr: CBCharacteristic?
    var subs = 0, ready = false, fatal = false
    var writePending = false, writeError: Error?
    var ff01: [Data] = [], ff03: [Data] = []
    var credits = 0
    var quietFF03 = false

    override init() { super.init(); cm = CBCentralManager(delegate: self, queue: nil) }
    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        if c.state == .poweredOn { c.scanForPeripherals(withServices: nil) }
        else if c.state == .unauthorized { print("Bluetooth unauthorized"); fatal = true }
    }
    func centralManager(_ c: CBCentralManager, didDiscover per: CBPeripheral, advertisementData ad: [String: Any], rssi: NSNumber) {
        let name = per.name ?? (ad[CBAdvertisementDataLocalNameKey] as? String) ?? ""
        guard name.lowercased().contains("seznik minix"), p == nil else { return }
        print("\(ts()) found \"\(name)\" rssi=\(rssi)")
        p = per; c.stopScan(); per.delegate = self; c.connect(per)
    }
    func centralManager(_ c: CBCentralManager, didConnect per: CBPeripheral) { print("\(ts()) connected"); per.discoverServices([svc]) }
    func centralManager(_ c: CBCentralManager, didFailToConnect per: CBPeripheral, error: Error?) { print("connect failed \(String(describing: error))"); fatal = true }
    func centralManager(_ c: CBCentralManager, didDisconnectPeripheral per: CBPeripheral, error: Error?) {
        print("\(ts()) disconnected \(error.map { "\($0)" } ?? "")"); if !ready { fatal = true }
    }
    func peripheral(_ per: CBPeripheral, didDiscoverServices e: Error?) {
        for s in per.services ?? [] where s.uuid == svc { per.discoverCharacteristics(nil, for: s) }
    }
    func peripheral(_ per: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error: Error?) {
        for ch in s.characteristics ?? [] {
            if ch.uuid == cWrite { wr = ch }
            if ch.uuid == cN1 || ch.uuid == cN3 { per.setNotifyValue(true, for: ch) }
        }
    }
    func peripheral(_ per: CBPeripheral, didUpdateNotificationStateFor ch: CBCharacteristic, error: Error?) {
        subs += 1; if subs == 2 { ready = true; print("\(ts()) subscribed FF01+FF03") }
    }
    func peripheral(_ per: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error: Error?) {
        let d = ch.value ?? Data()
        if ch.uuid == cN1 {
            ff01.append(d)
            let ascii = String(d.map { (0x20...0x7E).contains($0) ? Character(UnicodeScalar($0)) : "." })
            print("\(ts())   <- FF01 [\(hex(d))] \"\(ascii)\"")
        } else {
            ff03.append(d)
            if d.count == 2 && d[0] == 0x01 { credits += Int(d[1]) }
            if !quietFF03 { print("\(ts())   <- FF03 [\(hex(d))]") }
        }
    }
    func peripheral(_ per: CBPeripheral, didWriteValueFor ch: CBCharacteristic, error: Error?) { writeError = error; writePending = false }

    func write(_ bytes: [UInt8], label: String? = nil) -> Bool {
        guard let p = p, let wr = wr else { return false }
        if let l = label { print("\(ts()) -> \(l) [\(bytes.count <= 16 ? hex(Data(bytes)) : "\(bytes.count) bytes")]") }
        writePending = true; writeError = nil
        p.writeValue(Data(bytes), for: wr, type: .withResponse)
        let dl = Date().addingTimeInterval(5)
        while writePending && Date() < dl { spin(0.002) }
        if writePending { print("\(ts())   write timed out"); return false }
        if let e = writeError { print("\(ts())   write error: \(e)"); return false }
        return true
    }
}

func spin(_ s: Double) { RunLoop.current.run(until: Date().addingTimeInterval(s)) }
func waitFor(_ s: Double, _ cond: () -> Bool) -> Bool {
    let dl = Date().addingTimeInterval(s); while Date() < dl { if cond() { return true }; spin(0.01) }; return cond()
}

let pr = Printer()
guard waitFor(25, { pr.ready || pr.fatal }), pr.ready else { print("printer not ready; aborting"); exit(1) }
spin(1.0)

// Pre-flight: status must be clean
let n0 = pr.ff01.count
_ = pr.write([0x10, 0xFF, 0x40], label: "status 10 FF 40")
guard waitFor(2, { pr.ff01.count > n0 }), let st = pr.ff01.last?.last else { print("no status reply; aborting"); exit(2) }
let bad: [(UInt8, String)] = [(0x02, "cover open"), (0x04, "paper empty"), (0x10, "overheat"), (0x40, "overheat")]
let problems = bad.filter { st & $0.0 != 0 }.map { $0.1 }
if !problems.isEmpty { print("status 0x\(String(format: "%02X", st)) -> \(problems); aborting"); exit(3) }
print("\(ts()) status 0x\(String(format: "%02X", st)) OK, credits so far=\(pr.credits)")

// Print job
let creditsBefore = pr.credits
guard pr.write([UInt8](repeating: 0, count: 12), label: "wake 00x12"),
      pr.write([0x10, 0xFF, 0xF1, 0x03], label: "enable Lujiang 10 FF F1 03") else { exit(4) }
spin(0.05)

var job: [UInt8] = [0x1D, 0x76, 0x30, 0x00, UInt8(BPR & 0xFF), UInt8(BPR >> 8), UInt8(H & 0xFF), UInt8(H >> 8)]
job += bits
let chunk = 128
let nChunks = (job.count + chunk - 1) / chunk
print("\(ts()) -> GS v 0 header [\(hex(Data(job[0..<8])))] + \(bits.count) raster bytes in \(nChunks) x \(chunk)-byte chunks (write-with-response, 20 ms gap)")
pr.quietFF03 = true
let ff01BeforeRaster = pr.ff01.count
let tRaster = Date()
for i in 0..<nChunks {
    let slice = Array(job[(i * chunk)..<min(job.count, (i + 1) * chunk)])
    guard pr.write(slice) else { print("aborted at chunk \(i)"); exit(5) }
    spin(0.02)
}
print("\(ts()) raster sent in \(String(format: "%.2f", Date().timeIntervalSince(tRaster)))s; FF03 frames during raster: \(pr.ff03.count), credits granted since job start: \(pr.credits - creditsBefore)")
pr.quietFF03 = false

let gotOK = waitFor(10, { pr.ff01[ff01BeforeRaster...].contains { $0 == Data([0x4F, 0x4B]) } })
print("\(ts()) OK after raster: \(gotOK)")

_ = pr.write([0x1B, 0x4A, 0x50], label: "feed 80 dots 1B 4A 50")
_ = pr.write([0x10, 0xFF, 0xF1, 0x45], label: "end job 10 FF F1 45")
spin(3.0)

// Post-flight status
let n1 = pr.ff01.count
_ = pr.write([0x10, 0xFF, 0x40], label: "status 10 FF 40")
_ = waitFor(2, { pr.ff01.count > n1 })
spin(0.5)

let ff03Summary = Dictionary(grouping: pr.ff03.map { hex($0) }, by: { $0 }).map { "\($0.key) x\($0.value.count)" }.sorted()
print("\nFF03 frame histogram: \(ff03Summary)")
print("FF01 frames: \(pr.ff01.map { hex($0) })")
if let p = pr.p { pr.cm.cancelPeripheralConnection(p) }
spin(0.5)
print("done")
