import AppKit
import Foundation
import WebKit

enum BenchmarkRendererError: Error, CustomStringConvertible {
    case usage
    case readFailed(String)
    case javascriptFailed(String)
    case snapshotFailed(String)
    case writeFailed(String)

    var description: String {
        switch self {
        case .usage:
            return "usage: benchmark_renderer <html_path> <cases_json_path> <screenshot_path> <truths_json_path>"
        case .readFailed(let message):
            return "read failed: \(message)"
        case .javascriptFailed(let message):
            return "javascript failed: \(message)"
        case .snapshotFailed(let message):
            return "snapshot failed: \(message)"
        case .writeFailed(let message):
            return "write failed: \(message)"
        }
    }
}

private final class BenchmarkRenderer: NSObject, WKNavigationDelegate {
    private let html: String
    private let casesJSON: String
    private let screenshotPath: String
    private let truthsPath: String
    private let webView: WKWebView
    private var snapshotWidth: CGFloat = 1280
    private var snapshotHeight: CGFloat = 920

    init(html: String, casesJSON: String, screenshotPath: String, truthsPath: String) {
        self.html = html
        self.casesJSON = casesJSON
        self.screenshotPath = screenshotPath
        self.truthsPath = truthsPath
        self.webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 1280, height: 920))
        super.init()
        self.webView.navigationDelegate = self
    }

    func run() {
        webView.loadHTMLString(html, baseURL: nil)
        RunLoop.main.run()
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.10) {
            self.evaluateTruths()
        }
    }

    private func evaluateTruths() {
        let escapedCases = casesJSON
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "'", with: "\\'")
            .replacingOccurrences(of: "\n", with: "\\n")
        let script = """
        (() => {
            const cases = JSON.parse('\(escapedCases)');
            const doc = document.documentElement;
            const body = document.body;
            const pageWidth = Math.max(
                doc?.scrollWidth ?? 0,
                doc?.offsetWidth ?? 0,
                body?.scrollWidth ?? 0,
                body?.offsetWidth ?? 0,
                window.innerWidth ?? 0,
            );
            const pageHeight = Math.max(
                doc?.scrollHeight ?? 0,
                doc?.offsetHeight ?? 0,
                body?.scrollHeight ?? 0,
                body?.offsetHeight ?? 0,
                window.innerHeight ?? 0,
            );
            return JSON.stringify({
                page: {
                    width: Math.round(pageWidth),
                    height: Math.round(pageHeight),
                },
                truths: cases.map((testCase) => {
                    const benchmarkId = testCase.elementId ?? testCase.id;
                    const node = document.querySelector(`[data-benchmark-id="${benchmarkId}"]`);
                    if (!node) {
                        throw new Error(`Missing benchmark node for ${benchmarkId}`);
                    }
                    const rect = node.getBoundingClientRect();
                    return {
                        ...testCase,
                        box: {
                            x: Math.round(rect.left),
                            y: Math.round(rect.top),
                            width: Math.round(rect.width),
                            height: Math.round(rect.height),
                        },
                        point: {
                            x: Math.round(rect.left + (rect.width / 2)),
                            y: Math.round(rect.top + (rect.height / 2)),
                        },
                    };
                }),
            });
        })();
        """

        webView.evaluateJavaScript(script) { result, error in
            if let error {
                self.fail(.javascriptFailed(error.localizedDescription))
                return
            }
            guard let payloadJSON = result as? String else {
                self.fail(.javascriptFailed("renderer returned non-string truth payload"))
                return
            }
            do {
                let payloadData = Data(payloadJSON.utf8)
                let payload = try JSONSerialization.jsonObject(with: payloadData) as? [String: Any]
                let truths = payload?["truths"] ?? []
                if let page = payload?["page"] as? [String: Any] {
                    if let width = page["width"] as? NSNumber {
                        self.snapshotWidth = max(1, CGFloat(truncating: width))
                    }
                    if let height = page["height"] as? NSNumber {
                        self.snapshotHeight = max(1, CGFloat(truncating: height))
                    }
                }
                let truthsOnlyData = try JSONSerialization.data(
                    withJSONObject: ["truths": truths],
                    options: [.prettyPrinted]
                )
                try truthsOnlyData.write(
                    to: URL(fileURLWithPath: self.truthsPath),
                    options: .atomic
                )
            } catch {
                self.fail(.writeFailed(error.localizedDescription))
                return
            }
            self.webView.setFrameSize(
                NSSize(width: self.snapshotWidth, height: self.snapshotHeight)
            )
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                self.captureSnapshot()
            }
        }
    }

    private func captureSnapshot() {
        let config = WKSnapshotConfiguration()
        config.rect = NSRect(x: 0, y: 0, width: snapshotWidth, height: snapshotHeight)
        webView.takeSnapshot(with: config) { image, error in
            if let error {
                self.fail(.snapshotFailed(error.localizedDescription))
                return
            }
            guard
                let image,
                let tiff = image.tiffRepresentation,
                let rep = NSBitmapImageRep(data: tiff),
                let png = rep.representation(using: .png, properties: [:])
            else {
                self.fail(.snapshotFailed("unable to encode PNG"))
                return
            }
            do {
                try png.write(to: URL(fileURLWithPath: self.screenshotPath))
                exit(EXIT_SUCCESS)
            } catch {
                self.fail(.writeFailed(error.localizedDescription))
            }
        }
    }

    private func fail(_ error: BenchmarkRendererError) {
        fputs("\(error)\n", stderr)
        exit(EXIT_FAILURE)
    }
}

let args = CommandLine.arguments
guard args.count == 5 else {
    fputs("\(BenchmarkRendererError.usage)\n", stderr)
    exit(EXIT_FAILURE)
}

do {
    let html = try String(contentsOfFile: args[1], encoding: .utf8)
    let casesJSON = try String(contentsOfFile: args[2], encoding: .utf8)
    _ = NSApplication.shared
    NSApp.setActivationPolicy(.prohibited)
    BenchmarkRenderer(
        html: html,
        casesJSON: casesJSON,
        screenshotPath: args[3],
        truthsPath: args[4]
    ).run()
} catch {
    fputs("\(BenchmarkRendererError.readFailed(error.localizedDescription))\n", stderr)
    exit(EXIT_FAILURE)
}
