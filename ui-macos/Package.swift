// swift-tools-version: 6.1

import PackageDescription

var products: [Product] = [
    .library(name: "KassiberDaemonKit", targets: ["KassiberDaemonKit"]),
    .library(name: "KassiberViewModels", targets: ["KassiberViewModels"]),
]
var dependencies: [Package.Dependency] = []
var targets: [Target] = [
    .target(name: "KassiberDaemonKit"),
    .target(name: "KassiberViewModels", dependencies: ["KassiberDaemonKit"]),
    .testTarget(name: "KassiberDaemonKitTests", dependencies: ["KassiberDaemonKit"]),
    .testTarget(
        name: "KassiberViewModelsTests",
        dependencies: ["KassiberDaemonKit", "KassiberViewModels"]
    ),
]

#if !os(Linux)
products.append(.executable(name: "kassiber_native", targets: ["KassiberApp"]))
dependencies += [
    .package(url: "https://github.com/gonzalezreal/textual", from: "0.5.0"),
    .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.9.4"),
]
targets.append(
    .executableTarget(
        name: "KassiberApp",
        dependencies: [
            "KassiberDaemonKit",
            "KassiberViewModels",
            .product(name: "Textual", package: "textual"),
            .product(name: "Sparkle", package: "Sparkle"),
        ],
        resources: [.process("Resources")]
    )
)
#endif

let package = Package(
    name: "KassiberMacOS",
    defaultLocalization: "en",
    platforms: [
        // Textual 0.5 uses the macOS 15 attributed-text APIs.
        .macOS(.v15),
    ],
    products: products,
    dependencies: dependencies,
    targets: targets
)
