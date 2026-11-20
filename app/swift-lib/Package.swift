// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "swift-lib",
    platforms: [.macOS(.v11)],
    products: [
        .library(name: "swift-lib", type: .static, targets: ["swift-lib"]),
    ],
    dependencies: [
        .package(url: "https://github.com/Brendonovich/swift-rs", from: "1.0.6"),
    ],
    targets: [
        .target(
            name: "swift-lib",
            dependencies: [.product(name: "SwiftRs", package: "swift-rs")]
        ),
    ]
)
