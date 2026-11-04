import Foundation
import CoreBluetooth

// Scan BLE advertisements for N seconds; if a matching peripheral is found, connect and dump GATT.
let filter = (CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "").lowercased()
let seconds = CommandLine.arguments.count > 2 ? Double(CommandLine.arguments[2]) ?? 15 : 15

final class Scanner: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var cm: CBCentralManager!
    var seen = Set<UUID>()
    var target: CBPeripheral?
    var pendingChars = 0
    var finished = false

    override init() {
        super.init()
        cm = CBCentralManager(delegate: self, queue: nil)
    }

    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        print("central state: \(c.state.rawValue) (5=poweredOn, 3=unauthorized)")
        if c.state == .poweredOn {
            c.scanForPeripherals(withServices: nil, options: [CBCentralManagerScanOptionAllowDuplicatesKey: false])
        } else if c.state == .unauthorized {
            print("Bluetooth permission denied for this process"); finished = true
        }
    }

    func centralManager(_ c: CBCentralManager, didDiscover p: CBPeripheral, advertisementData ad: [String: Any], rssi: NSNumber) {
        guard !seen.contains(p.identifier) else { return }
        let name = p.name ?? (ad[CBAdvertisementDataLocalNameKey] as? String) ?? ""
        if !filter.isEmpty && !name.lowercased().contains(filter) { return }
        seen.insert(p.identifier)
        print("\n[BLE] name=\"\(name)\" id=\(p.identifier) rssi=\(rssi)")
        for (k, v) in ad { print("   \(k): \(v)") }
        if !filter.isEmpty && target == nil {
            target = p
            c.stopScan()
            print("   -> connecting to dump GATT…")
            p.delegate = self
            c.connect(p, options: nil)
        }
    }

    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        print("connected; discovering services")
        p.discoverServices(nil)
    }
    func centralManager(_ c: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) {
        print("failed to connect: \(String(describing: error))"); finished = true
    }

    func peripheral(_ p: CBPeripheral, didDiscoverServices error: Error?) {
        for s in p.services ?? [] {
            print("service \(s.uuid)")
            pendingChars += 1
            p.discoverCharacteristics(nil, for: s)
        }
        if (p.services ?? []).isEmpty { finished = true }
    }

    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error: Error?) {
        for ch in s.characteristics ?? [] {
            var props: [String] = []
            let pr = ch.properties
            if pr.contains(.read) { props.append("read") }
            if pr.contains(.write) { props.append("write") }
            if pr.contains(.writeWithoutResponse) { props.append("writeNoResp") }
            if pr.contains(.notify) { props.append("notify") }
            if pr.contains(.indicate) { props.append("indicate") }
            print("   [\(s.uuid)] char \(ch.uuid) props=\(props.joined(separator: ","))")
            if pr.contains(.read) { p.readValue(for: ch) }
        }
        pendingChars -= 1
        if pendingChars == 0 {
            print("   mtu(writeNoResp)=\(p.maximumWriteValueLength(for: .withoutResponse)) mtu(write)=\(p.maximumWriteValueLength(for: .withResponse))")
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { self.finished = true }
        }
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error: Error?) {
        let d = ch.value ?? Data()
        let hex = d.map { String(format: "%02x", $0) }.joined(separator: " ")
        let str = String(data: d, encoding: .utf8)?.trimmingCharacters(in: .controlCharacters) ?? ""
        print("   value \(ch.uuid): [\(hex)] \"\(str)\"")
    }
}

let s = Scanner()
let end = Date().addingTimeInterval(seconds)
while !s.finished && (Date() < end || s.target != nil) {
    RunLoop.current.run(until: Date().addingTimeInterval(0.2))
    if s.target != nil && Date() > end.addingTimeInterval(20) { print("GATT dump timed out"); break }
}
if let t = s.target { s.cm.cancelPeripheralConnection(t) }
print("\nscan done; matched \(s.seen.count) peripheral(s)")
