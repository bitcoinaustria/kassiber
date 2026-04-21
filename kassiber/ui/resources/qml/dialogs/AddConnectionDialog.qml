import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

// Add Connection wizard:
// step 1 = connection kind picker
// step 2 = XPub configuration mockup
Dialog {
    id: root

    property string currentStep: "picker"
    property string selectedKind: ""
    property bool previewSeeded: false

    property string connectionLabel: ""
    property string xpubValue: ""
    property string selectedAddressType: "p2wpkh"
    property string gapLimitValue: "10"
    property int backendIndex: 0

    readonly property bool isDetailStep: currentStep !== "picker"
    readonly property bool isXpubStep: currentStep === "xpub"
    readonly property bool canSubmitXpub: root.connectionLabel.trim().length > 0 && root.xpubValue.trim().length > 0
    readonly property var selectedKindMeta: root.kindMeta(root.selectedKind)
    readonly property string detectedKeyLabel: root.detectExtendedKeyPrefix(root.xpubValue)
    readonly property string fingerprintLabel: "\u2014"

    readonly property var sections: [
        {
            label: "Self-custody \u00b7 On-chain",
            items: [
                { id: "xpub",       name: "XPub",       desc: "Single-sig on-chain watch" },
                { id: "descriptor", name: "Descriptor", desc: "Multisig wallet descriptor" }
            ]
        },
        {
            label: "Lightning",
            items: [
                { id: "core-ln", name: "Core Lightning", desc: "CLN node RPC" },
                { id: "lnd",     name: "LND",            desc: "Lightning Network Daemon" },
                { id: "nwc",     name: "NWC",            desc: "Nostr Wallet Connect" }
            ]
        },
        {
            label: "Services \u00b7 Merchant",
            items: [
                { id: "btcpay", name: "BTCPay Server", desc: "Merchant API \u00b7 store read-key" },
                { id: "cashu",  name: "Cashu",         desc: "Ecash mint wallet" }
            ]
        },
        {
            label: "Exchanges \u00b7 Read-only API",
            items: [
                { id: "kraken",   name: "Kraken",   desc: "Read-only API key" },
                { id: "bitstamp", name: "Bitstamp", desc: "Read-only API key" },
                { id: "coinbase", name: "Coinbase", desc: "Read-only API key" },
                { id: "bitpanda", name: "Bitpanda", desc: "Read-only API key \u00b7 Austrian" },
                { id: "river",    name: "River",    desc: "Read-only API key" },
                { id: "strike",   name: "Strike",   desc: "Read-only API key" }
            ]
        },
        {
            label: "File",
            items: [
                { id: "csv",    name: "CSV import",     desc: "One-shot, from file" },
                { id: "bip329", name: "BIP-329 labels", desc: "Import labels \u00b7 JSONL" }
            ]
        }
    ]

    readonly property var xpubAddressTypes: [
        { id: "p2pkh",       label: "Pay to Public Key Hash",  example: "1A1zP1..." },
        { id: "p2sh-p2wpkh", label: "Pay to Script Hash",      example: "3J98t1..." },
        { id: "p2wpkh",      label: "Pay to Witness Pub Hash", example: "bc1qar..." },
        { id: "p2tr",        label: "Pay to Taproot",          example: "bc1p5c..." }
    ]

    readonly property var backendOptions: [
        { id: "mempool",  label: "Mempool.space (default)" },
        { id: "fulcrum",  label: "Fulcrum \u00b7 local Electrum" },
        { id: "esplora",  label: "Esplora \u00b7 custom URL" },
        { id: "bitcoinrpc", label: "Bitcoin Core RPC" }
    ]

    signal kindPicked(string kind)

    title: root.isDetailStep
        ? (root.selectedKindMeta["name"] || "Connection details")
        : "Add a connection"
    modal: true
    width: root.isDetailStep ? 888 : 760
    padding: 0
    standardButtons: Dialog.NoButton
    anchors.centerIn: parent
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    function detectExtendedKeyPrefix(value) {
        var normalized = String(value || "").trim().toLowerCase()
        var prefixes = ["xpub", "ypub", "zpub", "tpub", "upub", "vpub"]
        for (var i = 0; i < prefixes.length; i++) {
            if (normalized.indexOf(prefixes[i]) === 0) {
                return prefixes[i].toUpperCase()
            }
        }
        return normalized.length > 0 ? "UNKNOWN" : "\u2014"
    }

    function kindMeta(kind) {
        for (var i = 0; i < root.sections.length; i++) {
            var section = root.sections[i]
            for (var j = 0; j < section.items.length; j++) {
                var item = section.items[j]
                if (item.id === kind) {
                    return item
                }
            }
        }
        return { id: "", name: "", desc: "" }
    }

    function resetForm() {
        root.connectionLabel = ""
        root.xpubValue = ""
        root.selectedAddressType = "p2wpkh"
        root.gapLimitValue = "10"
        root.backendIndex = 0
    }

    function resetWizard() {
        root.currentStep = "picker"
        root.selectedKind = ""
        root.previewSeeded = false
        root.resetForm()
    }

    function seedXpubPreview() {
        root.previewSeeded = true
        root.connectionLabel = "ColdStorage"
        root.xpubValue = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4ogpiMZbpiaQL2j..."
        root.selectedAddressType = "p2wpkh"
        root.gapLimitValue = "10"
        root.backendIndex = 0
    }

    function openPicker() {
        root.resetWizard()
        root.open()
    }

    function openForKind(kind, usePreviewSeed) {
        root.resetWizard()
        root.selectedKind = kind
        if (kind === "xpub") {
            root.currentStep = "xpub"
            if (usePreviewSeed) {
                root.seedXpubPreview()
            }
        }
        root.open()
    }

    function goBack() {
        root.currentStep = "picker"
        root.previewSeeded = false
    }

    onKindPicked: {
        if (kind === "xpub") {
            root.selectedKind = kind
            root.currentStep = "xpub"
            root.previewSeeded = false
            root.resetForm()
        }
    }

    onClosed: root.resetWizard()

    background: Rectangle {
        color: Design.paper(theme)
        border.color: Design.ink(theme)
        border.width: 1
    }

    header: Rectangle {
        implicitHeight: root.isDetailStep ? 56 : 44
        color: Design.paper(theme)

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Design.line(theme)
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: root.isDetailStep ? theme.cardPadding + 10 : theme.cardPadding
            anchors.rightMargin: theme.cardPadding
            spacing: theme.spacingSm + 2

            Button {
                visible: root.isDetailStep
                flat: true
                padding: 0
                implicitWidth: 24
                implicitHeight: 24
                onClicked: root.goBack()

                contentItem: Text {
                    anchors.fill: parent
                    text: "\u2190"
                    color: Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontBodyStrong
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }

                background: Rectangle { color: "transparent" }
            }

            Text {
                text: root.title
                color: Design.ink(theme)
                font.family: Design.sans()
                font.pixelSize: root.isDetailStep ? theme.fontHeadingLg : theme.fontHeadingMd
                font.weight: Font.DemiBold
            }

            Item { Layout.fillWidth: true }

            Button {
                flat: true
                padding: 0
                implicitWidth: 24
                implicitHeight: 24
                onClicked: root.close()

                contentItem: Text {
                    anchors.fill: parent
                    text: "\u2715"
                    color: Design.ink2(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontCaption
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }

                background: Rectangle { color: "transparent" }
            }
        }
    }

    contentItem: Item {
        implicitWidth: stepLoader.item ? stepLoader.item.implicitWidth : 0
        implicitHeight: stepLoader.item ? stepLoader.item.implicitHeight : 0

        Loader {
            id: stepLoader
            anchors.fill: parent
            sourceComponent: root.isXpubStep ? xpubStep : pickerStep
        }
    }

    Component {
        id: pickerStep

        ColumnLayout {
            spacing: 0

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: intro.implicitHeight + 24
                color: Design.paper(theme)

                Text {
                    id: intro
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: theme.cardPadding + 4
                    anchors.rightMargin: theme.cardPadding + 4
                    text: "Kassiber is watch-only. Keys never leave your machine."
                    color: Design.ink2(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontBodyStrong
                    wrapMode: Text.WordWrap
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Design.line(theme)
                }
            }

            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: 440
                clip: true
                ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

                ColumnLayout {
                    width: root.availableWidth
                    spacing: theme.gridGap + 6

                    Repeater {
                        model: root.sections

                        delegate: ColumnLayout {
                            Layout.fillWidth: true
                            Layout.leftMargin: theme.cardPadding + 4
                            Layout.rightMargin: theme.cardPadding + 4
                            Layout.topMargin: index === 0 ? theme.gridGap : 0
                            spacing: theme.spacingSm + 2

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: theme.spacingSm + 2

                                Text {
                                    text: modelData.label.toUpperCase()
                                    color: Design.ink3(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontCaption
                                    font.weight: Font.Bold
                                    font.letterSpacing: 1.6
                                }

                                Rectangle {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 1
                                    color: Design.line(theme)
                                }

                                Text {
                                    text: (modelData.items.length < 10 ? "0" : "") + modelData.items.length
                                    color: Design.ink3(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontCaption
                                    font.weight: Font.Bold
                                    font.letterSpacing: 1.4
                                }
                            }

                            GridLayout {
                                Layout.fillWidth: true
                                columns: 2
                                columnSpacing: theme.spacingSm - 2
                                rowSpacing: theme.spacingSm - 2

                                Repeater {
                                    model: modelData.items

                                    KindPickerRow {
                                        Layout.fillWidth: true
                                        Layout.preferredWidth: 1
                                        Layout.preferredHeight: 60
                                        connectionName: modelData.name
                                        description: modelData.desc
                                        onClicked: root.kindPicked(modelData.id)
                                    }
                                }
                            }
                        }
                    }

                    Item { Layout.preferredHeight: theme.gridGap }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: noteContent.implicitHeight + 28
                color: Design.paperAlt(theme)

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    height: 1
                    color: Design.line(theme)
                }

                RowLayout {
                    id: noteContent
                    anchors.fill: parent
                    anchors.leftMargin: theme.cardPadding + 4
                    anchors.rightMargin: theme.cardPadding + 4
                    spacing: theme.spacingSm + 2

                    Text {
                        Layout.preferredWidth: 14
                        text: "\u25a1"
                        color: Design.accent(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodyStrong
                    }

                    Text {
                        Layout.fillWidth: true
                        text: "Watch-only by design. Kassiber imports history via extended public keys, descriptors, or read-only API credentials. No private keys or withdrawal permissions ever touch this machine through Kassiber."
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontBodySmall
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }
    }

    Component {
        id: xpubStep

        ColumnLayout {
            width: root.width
            spacing: 0

            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.cardPadding + 14
                Layout.rightMargin: theme.cardPadding + 14
                Layout.topMargin: theme.gridGap + 8
                Layout.bottomMargin: theme.gridGap + 6
                spacing: theme.gridGap + 4

                Text {
                    Layout.fillWidth: true
                    text: "Enter your extended public key. Kassiber will derive addresses and sync on-chain history."
                    color: Design.ink2(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontHeadingXs
                    wrapMode: Text.WordWrap
                }

                InputField {
                    Layout.fillWidth: true
                    label: "Connection label"
                    placeholderText: "e.g. Cold storage"
                    text: root.connectionLabel
                    onTextChanged: root.connectionLabel = text
                }

                InputField {
                    id: xpubInput
                    Layout.fillWidth: true
                    label: "XPub / YPub / ZPub"
                    mono: true
                    placeholderText: "xpub..."
                    rightText: "paste"
                    text: root.xpubValue
                    selectByMouse: true
                    onTextChanged: {
                        root.xpubValue = text
                        if (root.previewSeeded) {
                            cursorPosition = 0
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: theme.spacingSm + 8

                    Text {
                        text: "Detected: " + root.detectedKeyLabel
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                    }

                    Text {
                        text: "Fingerprint: " + root.fingerprintLabel
                        color: Design.ink3(theme)
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontBodySmall
                    }
                }

                Text {
                    Layout.fillWidth: true
                    text: "ADDRESS TYPES TO DERIVE"
                    color: Design.ink2(theme)
                    font.family: Design.sans()
                    font.pixelSize: theme.fontCaption
                    font.weight: Font.DemiBold
                    font.capitalization: Font.AllUppercase
                    font.letterSpacing: 1.4
                }

                GridLayout {
                    Layout.fillWidth: true
                    columns: 2
                    columnSpacing: theme.spacingSm + 6
                    rowSpacing: theme.spacingSm

                    Repeater {
                        model: root.xpubAddressTypes

                        delegate: Button {
                            id: addressTypeButton
                            Layout.fillWidth: true
                            Layout.preferredWidth: 1
                            implicitHeight: 52
                            flat: true
                            padding: 0
                            hoverEnabled: true
                            onClicked: root.selectedAddressType = modelData.id

                            contentItem: RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: 14
                                anchors.rightMargin: 14
                                spacing: theme.spacingSm + 4

                                Rectangle {
                                    width: 18
                                    height: 18
                                    radius: 2
                                    border.color: root.selectedAddressType === modelData.id ? Design.accent(theme) : Design.line2(theme)
                                    border.width: 1
                                    color: root.selectedAddressType === modelData.id ? Design.accent(theme) : "transparent"

                                    Text {
                                        anchors.centerIn: parent
                                        text: "\u2713"
                                        visible: root.selectedAddressType === modelData.id
                                        color: Design.paper(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontCaption
                                    }
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: modelData.label
                                    color: Design.ink(theme)
                                    font.family: Design.sans()
                                    font.pixelSize: theme.fontBodyStrong
                                    elide: Text.ElideRight
                                }

                                Text {
                                    text: modelData.example
                                    color: Design.ink3(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontBodySmall
                                }
                            }

                            background: Rectangle {
                                color: root.selectedAddressType === modelData.id
                                    ? Design.paperAlt(theme)
                                    : (addressTypeButton.hovered ? Design.paper(theme) : "transparent")
                                border.color: root.selectedAddressType === modelData.id ? Design.ink(theme) : Design.line(theme)
                                border.width: 1
                            }
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: theme.spacingSm + 6

                    InputField {
                        Layout.fillWidth: true
                        Layout.preferredWidth: 1
                        label: "Gap limit"
                        mono: true
                        text: root.gapLimitValue
                        validator: IntValidator { bottom: 1; top: 999 }
                        onTextChanged: root.gapLimitValue = text
                    }

                    Control {
                        Layout.fillWidth: true
                        Layout.preferredWidth: 1
                        implicitHeight: backendColumn.implicitHeight

                        contentItem: Column {
                            id: backendColumn
                            spacing: 6

                            Text {
                                text: "Sync backend"
                                color: Design.ink2(theme)
                                font.family: Design.sans()
                                font.pixelSize: 10
                                font.weight: Font.DemiBold
                                font.capitalization: Font.AllUppercase
                                font.letterSpacing: 1.4
                            }

                            ComboBox {
                                id: backendCombo
                                width: parent.width
                                implicitHeight: 36
                                model: root.backendOptions
                                textRole: "label"
                                currentIndex: root.backendIndex
                                onActivated: root.backendIndex = currentIndex

                                contentItem: Text {
                                    leftPadding: 12
                                    rightPadding: 34
                                    verticalAlignment: Text.AlignVCenter
                                    text: backendCombo.displayText
                                    color: Design.ink(theme)
                                    font.family: Design.sans()
                                    font.pixelSize: theme.fontBodyStrong
                                    elide: Text.ElideRight
                                }

                                background: Rectangle {
                                    color: Design.paperAlt(theme)
                                    border.color: backendCombo.popup.visible ? Design.ink(theme) : Design.line(theme)
                                    border.width: 1
                                }

                                indicator: Text {
                                    x: backendCombo.width - width - 12
                                    y: (backendCombo.height - height) / 2
                                    text: "\u2304"
                                    color: Design.ink3(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontBodyStrong
                                }

                                delegate: ItemDelegate {
                                    id: backendOption
                                    required property var modelData
                                    required property int index

                                    width: backendCombo.width
                                    padding: 0
                                    highlighted: backendCombo.highlightedIndex === index
                                    onClicked: {
                                        backendCombo.currentIndex = index
                                        backendCombo.popup.close()
                                    }

                                    contentItem: Text {
                                        leftPadding: 12
                                        rightPadding: 12
                                        verticalAlignment: Text.AlignVCenter
                                        text: modelData.label
                                        color: Design.ink(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontBodyStrong
                                        elide: Text.ElideRight
                                    }

                                    background: Rectangle {
                                        color: backendOption.highlighted ? Design.paperAlt(theme) : Design.paper(theme)
                                        border.width: 0
                                    }
                                }

                                popup: Popup {
                                    y: backendCombo.height - 1
                                    width: backendCombo.width
                                    padding: 0
                                    implicitHeight: Math.min(contentItem.implicitHeight, 180)

                                    contentItem: ListView {
                                        clip: true
                                        implicitHeight: contentHeight
                                        model: backendCombo.delegateModel
                                        currentIndex: backendCombo.highlightedIndex
                                        ScrollBar.vertical: ScrollBar { }
                                    }

                                    background: Rectangle {
                                        color: Design.paper(theme)
                                        border.color: Design.ink(theme)
                                        border.width: 1
                                    }
                                }
                            }
                        }
                    }
                }

                Text {
                    Layout.fillWidth: true
                    text: "Phase 1 shell preview only. Wallet creation and sync still happen through the CLI today."
                    color: Design.ink3(theme)
                    font.family: Design.mono(theme)
                    font.pixelSize: theme.fontCaption
                    wrapMode: Text.WordWrap
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 1
                color: Design.line(theme)
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: theme.cardPadding + 14
                Layout.rightMargin: theme.cardPadding + 14
                Layout.topMargin: theme.gridGap + 2
                Layout.bottomMargin: theme.gridGap + 8
                spacing: theme.spacingSm + 4

                Item { Layout.fillWidth: true }

                ActionButton {
                    variant: "ghost"
                    size: "md"
                    text: "Cancel"
                    onClicked: root.close()
                }

                ActionButton {
                    variant: "primary"
                    size: "lg"
                    text: "\u2713  Save and sync"
                    enabled: root.canSubmitXpub
                    onClicked: root.close()
                }
            }
        }
    }
}
