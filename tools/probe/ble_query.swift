import Foundation
import CoreBluetooth

// Connect to the printer over BLE (service FF00), subscribe to FF01/FF03,
// send read-only LuckPrinter info queries to FF02 and print every notification.
// NOTE: no print / feed commands are sent.

let nameFilter = "seznik minix"
let svc = CBUUID(string: "FF00")
let cWrite = CBUUID(string: "FF02")
let cN1 = CBUUID(string: "FF01")
let cN3 = CBUUID(string: "FF03")

let queries: [(String, [UInt8])] = [
    ("model        10 FF 20 F0", [0x10, 0xFF, 0x20, 0xF0]),
    ("firmware     10 FF 20 F1", [0x10, 0xFF, 0x20, 0xF1]),
    ("serial       10 FF 20 F2", [0x10, 0xFF, 0x20, 0xF2]),
    ("all-info     10 FF 70",    [0x10, 0xFF, 0x70]),
    ("battery      10 FF 50 F1", [0x10, 0xFF, 0x50, 0xF1]),
    ("status       10 FF 40",    [0x10, 0xFF, 0x40]),
    ("density?     10 FF 11",    [0x10, 0xFF, 0x11]),
    ("autoshutdn?  10 FF 13",    [0x10, 0xFF, 0x13]),
    ("speed?       10 FF 20 A0", [0x10, 0xFF, 0x20, 0xA0]),
]

func fmt(_ d: Data) -> String {
    let hex = d.map { String(format: "%02X", $0) }.joined(separator: " ")
    let ascii = String(d.map { (0x20...0x7E).contains($0) ? Character(UnicodeScalar($0)) : "." })
    return "[\(hex)]  \"\(ascii)\""
}

final class Q: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var cm: CBCentralManager!
    var p: CBPeripheral?
    var wr: CBCharacteristic?
    var subscribed = 0
    var ready = false
    var done = false
    let t0 = Date()
    func ts() -> String { String(format: "%6.3fs", Date().timeIntervalSince(t0)) }

    override init() { super.init(); cm = CBCentralManager(delegate: self, queue: nil) }

    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        if c.state == .poweredOn { c.scanForPeripherals(withServices: nil) }
        else if c.state == .unauthorized { print("unauthorized"); done = true }
    }
    func centralManager(_ c: CBCentralManager, didDiscover per: CBPeripheral, advertisementData ad: [String: Any], rssi: NSNumber) {
        let name = per.name ?? (ad[CBAdvertisementDataLocalNameKey] as? String) ?? ""
        guard name.lowercased().contains(nameFilter), p == nil else { return }
        print("\(ts()) found \"\(name)\" rssi=\(rssi) id=\(per.identifier)")
        p = per; c.stopScan(); per.delegate = self; c.connect(per)
    }
    func centralManager(_ c: CBCentralManager, didConnect per: CBPeripheral) {
        print("\(ts()) connected"); per.discoverServices([svc])
    }
    func centralManager(_ c: CBCentralManager, didDisconnectPeripheral per: CBPeripheral, error: Error?) {
        print("\(ts()) disconnected \(String(describing: error))")
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
        print("\(ts()) notify on \(ch.uuid) = \(ch.isNotifying) \(error.map { "err \($0)" } ?? "")")
        subscribed += 1
        if subscribed == 2 { ready = true }
    }
    func peripheral(_ per: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error: Error?) {
        print("\(ts())   <- \(ch.uuid): \(fmt(ch.value ?? Data()))")
    }
    func peripheral(_ per: CBPeripheral, didWriteValueFor ch: CBCharacteristic, error: Error?) {
        if let e = error { print("\(ts())   write error: \(e)") }
    }
    func send(_ label: String, _ bytes: [UInt8]) {
        guard let p = p, let wr = wr else { return }
        print("\n\(ts()) -> \(label)")
        p.writeValue(Data(bytes), for: wr, type: .withResponse)
    }
}

func spin(_ s: Double) { RunLoop.current.run(until: Date().addingTimeInterval(s)) }

let q = Q()
let deadline = Date().addingTimeInterval(25)
while !q.ready && !q.done && Date() < deadline { spin(0.1) }
guard q.ready else { print("never became ready"); exit(1) }
print("\(q.ts()) listening 1.5s for connect-time frames…")
spin(1.5)
for (label, bytes) in queries { q.send(label, bytes); spin(1.2) }
spin(1.0)
if let p = q.p { q.cm.cancelPeripheralConnection(p) }
spin(0.5)
print("\ndone")
