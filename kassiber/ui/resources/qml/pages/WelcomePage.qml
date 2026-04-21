import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "../components"
import "../components/Design.js" as Design

// Static 3-step Welcome flow (Intro \u2192 Setup \u2192 Encrypt). All step state
// and transitions live locally \u2014 no view-model bindings yet.
Item {
    id: root

    // Step machine
    property string step: "intro"   // "intro" | "setup" | "encrypt"

    // Form state (not wired to anything yet)
    property string userName: ""
    property string workspaceName: "My Books"
    property string residency: "AT"

    property string encryptMode: "encrypt"  // "encrypt" | "plain"
    property string passphrase: ""
    property string confirmPassphrase: ""
    property bool showPassphrase: false

    readonly property bool passphraseMatches: passphrase === confirmPassphrase
    readonly property bool passphraseLongEnough: passphrase.length >= 12
    readonly property bool canFinish: encryptMode === "plain"
        || (passphraseLongEnough && passphraseMatches)

    readonly property var residencyOptions: [
        { code: "AT",    enabled: true,  label: "AT" },
        { code: "DE",    enabled: false, label: "DE" },
        { code: "CH",    enabled: false, label: "CH" },
        { code: "EU",    enabled: false, label: "EU" },
        { code: "Other", enabled: false, label: "Other" }
    ]

    readonly property var manifestoFacts: [
        { n: "01", heading: "Local",          body: "Data lives on your disk. Plain files. Export any time." },
        { n: "02", heading: "Watch-only",     body: "xpubs, descriptors & LN read-keys. Never private keys." },
        { n: "03", heading: "Austrian-ready", body: "FIFO \u00b7 \u00a727a EStG \u00b7 KESt 27,5 % \u2014 built in." },
        { n: "04", heading: "Encrypted",      body: "Optional at-rest encryption with a passphrase only you know." }
    ]

    Rectangle {
        anchors.fill: parent
        color: Design.paper(theme)
    }

    // ---------------------------------------------------------------------
    // Promise bar (always visible)
    // ---------------------------------------------------------------------

    Rectangle {
        id: promiseBar
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 34
        color: Design.paperAlt(theme)

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Design.ink(theme)
        }

        Row {
            anchors.centerIn: parent
            spacing: theme.spacingSm + 2

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: 6
                height: 6
                radius: 3
                color: Design.accent(theme)
            }

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: "WATCH-ONLY"
                color: Design.ink2(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontCaption
                font.weight: Font.Bold
                font.letterSpacing: 2.0
            }

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: "\u00b7"
                color: Design.ink3(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontCaption
            }

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: "THIS APP NEVER TOUCHES YOUR PRIVATE KEYS."
                color: Design.ink2(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontCaption
                font.letterSpacing: 1.8
            }
        }
    }

    // ---------------------------------------------------------------------
    // Step indicator (only on setup / encrypt)
    // ---------------------------------------------------------------------

    Rectangle {
        id: stepIndicator
        visible: root.step !== "intro"
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: promiseBar.bottom
        height: 42
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
            anchors.leftMargin: 28
            anchors.rightMargin: 28
            spacing: theme.spacingSm + 6

            Repeater {
                model: [
                    { id: "setup",   n: "01", label: "Identity" },
                    { id: "encrypt", n: "02", label: "Encryption" }
                ]

                delegate: RowLayout {
                    spacing: theme.spacingSm
                    property bool active: root.step === modelData.id
                    property bool done: (modelData.id === "setup" && root.step === "encrypt")

                    Rectangle {
                        Layout.preferredWidth: 20
                        Layout.preferredHeight: 20
                        color: "transparent"
                        border.color: active
                            ? Design.accent(theme)
                            : (done ? Design.ink2(theme) : Design.ink3(theme))
                        border.width: 1

                        Text {
                            anchors.centerIn: parent
                            text: done ? "\u2713" : modelData.n
                            color: active
                                ? Design.accent(theme)
                                : (done ? Design.ink2(theme) : Design.ink3(theme))
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontMicro
                            font.weight: Font.Bold
                        }
                    }

                    Text {
                        text: modelData.label.toUpperCase()
                        color: active
                            ? Design.ink(theme)
                            : (done ? Design.ink2(theme) : Design.ink3(theme))
                        font.family: Design.mono(theme)
                        font.pixelSize: theme.fontCaption
                        font.letterSpacing: 1.4
                    }
                }
            }

            Item { Layout.fillWidth: true }

            Text {
                text: "SETUP"
                color: Design.ink3(theme)
                font.family: Design.mono(theme)
                font.pixelSize: theme.fontCaption
                font.letterSpacing: 1.4
            }
        }
    }

    // ---------------------------------------------------------------------
    // Body (switches on step)
    // ---------------------------------------------------------------------

    Item {
        id: body
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: stepIndicator.visible ? stepIndicator.bottom : promiseBar.bottom
        anchors.bottom: parent.bottom

        // ========== STEP 1 — INTRO ==========================================
        Item {
            anchors.fill: parent
            visible: root.step === "intro"

            GridLayout {
                anchors.fill: parent
                anchors.leftMargin: 72
                anchors.rightMargin: 64
                anchors.topMargin: 60
                anchors.bottomMargin: 40
                columns: 2
                columnSpacing: 60
                rowSpacing: 0

                // Left column: headline + body + CTA
                ColumnLayout {
                    Layout.alignment: Qt.AlignVCenter
                    Layout.horizontalStretchFactor: 14
                    Layout.minimumWidth: 440
                    Layout.fillWidth: true
                    spacing: 28

                    Text {
                        Layout.fillWidth: true
                        Layout.maximumWidth: 640
                        text: "Your books.<br/><i>Your keys.</i>"
                        textFormat: Text.StyledText
                        color: Design.ink(theme)
                        font.family: Design.sans()
                        font.pixelSize: 112
                        font.weight: Font.DemiBold
                        font.letterSpacing: -3.0
                        lineHeight: 0.92
                        wrapMode: Text.WordWrap
                    }

                    Text {
                        Layout.fillWidth: true
                        Layout.maximumWidth: 520
                        text: "Kassiber keeps every satoshi in a ledger you own \u2014 on your machine, " +
                              "in plain files, verifiable by you and no one else. No cloud. No custodian. No breach-in-waiting."
                        color: Design.ink2(theme)
                        font.family: Design.sans()
                        font.pixelSize: theme.fontHeadingXs + 3
                        lineHeight: 1.55
                        wrapMode: Text.WordWrap
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 18

                        ActionButton {
                            variant: "primary"
                            size: "lg"
                            text: "\u2192  Open the ledger"
                            onClicked: root.step = "setup"
                        }

                        Text {
                            Layout.alignment: Qt.AlignVCenter
                            text: "TWO-MINUTE SETUP  \u00b7  NO ACCOUNT"
                            color: Design.ink3(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.letterSpacing: 1.6
                        }
                    }
                }

                // Right column: numbered facts
                ColumnLayout {
                    Layout.alignment: Qt.AlignVCenter
                    Layout.fillWidth: true
                    Layout.horizontalStretchFactor: 10
                    Layout.minimumWidth: 300
                    spacing: 28

                    Repeater {
                        model: root.manifestoFacts

                        delegate: ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 6

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 16

                                Column {
                                    Layout.preferredWidth: 36
                                    spacing: 2

                                    Rectangle {
                                        width: 20
                                        height: 1
                                        color: Design.accent(theme)
                                    }

                                    Text {
                                        text: modelData.n
                                        color: Design.accent(theme)
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontBodySmall
                                        font.weight: Font.Bold
                                        font.letterSpacing: 1.6
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 4

                                    Text {
                                        text: modelData.heading
                                        color: Design.ink(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontHeadingMd
                                        font.weight: Font.DemiBold
                                        font.letterSpacing: -0.2
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData.body
                                        color: Design.ink3(theme)
                                        font.family: Design.sans()
                                        font.pixelSize: theme.fontBodyStrong
                                        lineHeight: 1.5
                                        wrapMode: Text.WordWrap
                                    }
                                }
                            }

                            Rectangle {
                                Layout.fillWidth: true
                                Layout.preferredHeight: 1
                                color: Design.line(theme)
                            }
                        }
                    }
                }
            }
        }

        // ========== STEP 2 — SETUP ==========================================
        Item {
            anchors.fill: parent
            visible: root.step === "setup"

            RowLayout {
                anchors.fill: parent
                spacing: 0

                // Left context
                Rectangle {
                    Layout.fillHeight: true
                    Layout.fillWidth: true
                    Layout.horizontalStretchFactor: 1
                    Layout.minimumWidth: 420
                    color: Design.paperAlt(theme)

                    Rectangle {
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        width: 1
                        color: Design.line(theme)
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 56
                        anchors.rightMargin: 56
                        anchors.topMargin: 48
                        anchors.bottomMargin: 40
                        spacing: 16

                        Text {
                            text: "STEP 01 OF 02"
                            color: Design.accent(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.letterSpacing: 2.0
                            font.weight: Font.Bold
                        }

                        Text {
                            Layout.fillWidth: true
                            text: "Tell us<br/>who's writing."
                            textFormat: Text.StyledText
                            color: Design.ink(theme)
                            font.family: Design.sans()
                            font.pixelSize: 56
                            font.weight: Font.DemiBold
                            font.letterSpacing: -1.5
                            lineHeight: 0.98
                        }

                        Text {
                            Layout.fillWidth: true
                            Layout.maximumWidth: 380
                            text: "Your name and workspace live only on this device. The workspace becomes " +
                                  "a folder of plain files \u2014 you can rename, move, or delete it at any time."
                            color: Design.ink2(theme)
                            font.family: Design.sans()
                            font.pixelSize: theme.fontHeadingXs
                            lineHeight: 1.55
                            wrapMode: Text.WordWrap
                        }

                        Item { Layout.fillHeight: true }

                        Column {
                            Layout.fillWidth: true
                            spacing: 4

                            Text {
                                text: "// STORED AT"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.letterSpacing: 1.4
                            }

                            Text {
                                text: "~/.kassiber/" + root.workspaceName.toLowerCase().replace(/\s+/g, '-') + "/"
                                color: Design.ink2(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.letterSpacing: 1.2
                            }
                        }
                    }
                }

                // Right form
                Rectangle {
                    Layout.fillHeight: true
                    Layout.fillWidth: true
                    Layout.horizontalStretchFactor: 1
                    Layout.minimumWidth: 420
                    color: Design.paper(theme)

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 56
                        anchors.rightMargin: 56
                        anchors.topMargin: 48
                        anchors.bottomMargin: 40
                        spacing: 18

                        InputField {
                            Layout.fillWidth: true
                            label: "Your name"
                            placeholderText: "e.g. Alice"
                            text: root.userName
                            onEditingFinished: root.userName = fieldItem.text
                        }

                        InputField {
                            Layout.fillWidth: true
                            label: "Workspace name"
                            placeholderText: "My Books"
                            text: root.workspaceName
                            onEditingFinished: root.workspaceName = fieldItem.text
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 8

                            Text {
                                text: "TAX RESIDENCY"
                                color: Design.ink2(theme)
                                font.family: Design.sans()
                                font.pixelSize: theme.fontCaption
                                font.weight: Font.DemiBold
                                font.letterSpacing: 1.4
                            }

                            Flow {
                                Layout.fillWidth: true
                                spacing: theme.spacingXs + 2

                                Repeater {
                                    model: root.residencyOptions

                                    Button {
                                        property bool selected: root.residency === modelData.code
                                        enabled: modelData.enabled
                                        flat: true
                                        padding: 0
                                        implicitHeight: 28
                                        implicitWidth: label.implicitWidth + 32
                                        onClicked: if (modelData.enabled) root.residency = modelData.code

                                        contentItem: Text {
                                            id: label
                                            anchors.fill: parent
                                            text: modelData.label
                                            color: parent.selected
                                                ? Design.paper(theme)
                                                : (parent.enabled ? Design.ink(theme) : Design.ink3(theme))
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontBodySmall
                                            font.weight: Font.Bold
                                            font.letterSpacing: 1.2
                                            font.capitalization: Font.AllUppercase
                                            font.strikeout: !parent.enabled
                                            horizontalAlignment: Text.AlignHCenter
                                            verticalAlignment: Text.AlignVCenter
                                        }

                                        background: Rectangle {
                                            color: parent.selected ? Design.ink(theme) : "transparent"
                                            border.color: parent.selected ? Design.ink(theme) : Design.line(theme)
                                            border.width: 1
                                            opacity: parent.enabled ? 1.0 : 0.55
                                        }
                                    }
                                }
                            }

                            Text {
                                Layout.fillWidth: true
                                text: "Only Austria is available at launch. Other jurisdictions coming soon."
                                color: Design.ink3(theme)
                                font.family: Design.sans()
                                font.pixelSize: theme.fontBodySmall
                                font.italic: true
                                wrapMode: Text.WordWrap
                            }
                        }

                        Item { Layout.fillHeight: true }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 1
                            color: Design.ink(theme)
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            Layout.topMargin: 6

                            Button {
                                flat: true
                                padding: 0
                                onClicked: root.step = "intro"
                                implicitHeight: 24

                                contentItem: Text {
                                    text: "\u2190 BACK"
                                    color: Design.ink2(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontBodySmall
                                    font.letterSpacing: 1.4
                                }

                                background: Rectangle { color: "transparent" }
                            }

                            Item { Layout.fillWidth: true }

                            ActionButton {
                                variant: "primary"
                                size: "lg"
                                text: "\u2192  Continue"
                                onClicked: root.step = "encrypt"
                            }
                        }
                    }
                }
            }
        }

        // ========== STEP 3 — ENCRYPT ========================================
        Item {
            anchors.fill: parent
            visible: root.step === "encrypt"

            RowLayout {
                anchors.fill: parent
                spacing: 0

                // Left context
                Rectangle {
                    Layout.fillHeight: true
                    Layout.fillWidth: true
                    Layout.horizontalStretchFactor: 1
                    Layout.minimumWidth: 420
                    color: Design.paperAlt(theme)

                    Rectangle {
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        width: 1
                        color: Design.line(theme)
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 56
                        anchors.rightMargin: 56
                        anchors.topMargin: 48
                        anchors.bottomMargin: 40
                        spacing: 16

                        Text {
                            text: "STEP 02 OF 02"
                            color: Design.accent(theme)
                            font.family: Design.mono(theme)
                            font.pixelSize: theme.fontCaption
                            font.letterSpacing: 2.0
                            font.weight: Font.Bold
                        }

                        Text {
                            Layout.fillWidth: true
                            text: "Lock the<br/><i>door.</i>"
                            textFormat: Text.StyledText
                            color: Design.ink(theme)
                            font.family: Design.sans()
                            font.pixelSize: 56
                            font.weight: Font.DemiBold
                            font.letterSpacing: -1.5
                            lineHeight: 0.98
                        }

                        Text {
                            Layout.fillWidth: true
                            Layout.maximumWidth: 380
                            text: "Kassiber can encrypt your database file at rest with a passphrase only you know. " +
                                  "Anyone with your disk would see opaque ciphertext \u2014 not balances, not addresses, not tags."
                            color: Design.ink2(theme)
                            font.family: Design.sans()
                            font.pixelSize: theme.fontHeadingXs
                            lineHeight: 1.55
                            wrapMode: Text.WordWrap
                        }

                        // Warning box
                        Rectangle {
                            Layout.fillWidth: true
                            Layout.maximumWidth: 380
                            Layout.preferredHeight: warnBody.implicitHeight + theme.cardPadding * 2
                            color: Design.paper(theme)
                            border.color: Design.line(theme)
                            border.width: 1

                            RowLayout {
                                anchors.fill: parent
                                anchors.margins: theme.cardPadding - 2
                                spacing: theme.spacingSm

                                Text {
                                    text: "\u26a0"
                                    color: Design.accent(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontBodyStrong
                                    font.weight: Font.Bold
                                }

                                Text {
                                    id: warnBody
                                    Layout.fillWidth: true
                                    text: "<b>We can't recover it.</b> Kassiber never sees your passphrase. If you lose it, the encrypted workspace is unreadable \u2014 including by us. Write it down."
                                    textFormat: Text.StyledText
                                    color: Design.ink2(theme)
                                    font.family: Design.sans()
                                    font.pixelSize: theme.fontBody
                                    lineHeight: 1.55
                                    wrapMode: Text.WordWrap
                                }
                            }
                        }

                        Item { Layout.fillHeight: true }

                        Column {
                            Layout.fillWidth: true
                            spacing: 4

                            Text {
                                text: "// CIPHER"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.letterSpacing: 1.4
                            }

                            Text {
                                text: "AES-256-GCM"
                                color: Design.ink2(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.letterSpacing: 1.2
                            }

                            Text {
                                Layout.topMargin: 6
                                text: "// KEY DERIVATION"
                                color: Design.ink3(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.letterSpacing: 1.4
                            }

                            Text {
                                text: "Argon2id \u00b7 256 MB \u00b7 3 passes"
                                color: Design.ink2(theme)
                                font.family: Design.mono(theme)
                                font.pixelSize: theme.fontCaption
                                font.letterSpacing: 1.2
                            }
                        }
                    }
                }

                // Right form
                Rectangle {
                    Layout.fillHeight: true
                    Layout.fillWidth: true
                    Layout.horizontalStretchFactor: 1
                    Layout.minimumWidth: 420
                    color: Design.paper(theme)

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 56
                        anchors.rightMargin: 56
                        anchors.topMargin: 48
                        anchors.bottomMargin: 40
                        spacing: 16

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: theme.spacingSm + 2

                            ChoiceCard {
                                Layout.fillWidth: true
                                Layout.fillHeight: false
                                letter: "A"
                                title: "Encrypt"
                                tagline: "Recommended"
                                description: "Passphrase required to open the workspace. Data at rest is unreadable without it."
                                active: root.encryptMode === "encrypt"
                                onClicked: root.encryptMode = "encrypt"
                            }

                            ChoiceCard {
                                Layout.fillWidth: true
                                letter: "B"
                                title: "Plain"
                                tagline: "Insecure \u00b7 not recommended"
                                warning: true
                                description: "Debug / evaluation only. Your database is written in the clear \u2014 anyone with disk access can read every balance, address and tag."
                                active: root.encryptMode === "plain"
                                onClicked: root.encryptMode = "plain"
                            }
                        }

                        // Passphrase box (only in encrypt mode)
                        Rectangle {
                            visible: root.encryptMode === "encrypt"
                            Layout.fillWidth: true
                            Layout.preferredHeight: pwCol.implicitHeight + theme.cardPadding * 2
                            color: Design.paperAlt(theme)
                            border.color: Design.ink(theme)
                            border.width: 1

                            ColumnLayout {
                                id: pwCol
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                anchors.leftMargin: theme.cardPadding + 2
                                anchors.rightMargin: theme.cardPadding + 2
                                anchors.topMargin: theme.cardPadding + 2
                                spacing: theme.spacingSm + 4

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: theme.spacingSm

                                    InputField {
                                        Layout.fillWidth: true
                                        label: "Passphrase"
                                        placeholderText: "at least 12 characters"
                                        mono: true
                                        echoMode: root.showPassphrase ? TextInput.Normal : TextInput.Password
                                        text: root.passphrase
                                        onEditingFinished: root.passphrase = fieldItem.text
                                    }

                                    InputField {
                                        Layout.fillWidth: true
                                        label: "Confirm passphrase"
                                        placeholderText: "repeat"
                                        mono: true
                                        echoMode: root.showPassphrase ? TextInput.Normal : TextInput.Password
                                        text: root.confirmPassphrase
                                        onEditingFinished: root.confirmPassphrase = fieldItem.text
                                    }
                                }

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: theme.spacingSm

                                    Text {
                                        Layout.fillWidth: true
                                        text: !root.passphrase.length
                                            ? "\u2014 NONE \u2014"
                                            : (!root.passphraseLongEnough
                                                ? "AT LEAST 12 CHARACTERS REQUIRED \u00b7 " + root.passphrase.length + "/12"
                                                : (!root.passphraseMatches
                                                    ? "PASSPHRASES DON'T MATCH."
                                                    : "OK"))
                                        color: root.passphrase.length && root.passphraseLongEnough && root.passphraseMatches
                                            ? theme.positive
                                            : (root.passphrase.length ? Design.accent(theme) : Design.ink3(theme))
                                        font.family: Design.mono(theme)
                                        font.pixelSize: theme.fontCaption
                                        font.letterSpacing: 1.2
                                    }

                                    Button {
                                        flat: true
                                        padding: 0
                                        implicitHeight: 22
                                        implicitWidth: showText.implicitWidth + 18
                                        onClicked: root.showPassphrase = !root.showPassphrase

                                        contentItem: Text {
                                            id: showText
                                            anchors.fill: parent
                                            text: root.showPassphrase ? "HIDE" : "SHOW"
                                            color: Design.ink2(theme)
                                            font.family: Design.mono(theme)
                                            font.pixelSize: theme.fontCaption
                                            font.letterSpacing: 1.4
                                            horizontalAlignment: Text.AlignHCenter
                                            verticalAlignment: Text.AlignVCenter
                                        }

                                        background: Rectangle {
                                            color: "transparent"
                                            border.color: Design.line(theme)
                                            border.width: 1
                                        }
                                    }
                                }
                            }
                        }

                        // Plain-mode warning
                        Rectangle {
                            visible: root.encryptMode === "plain"
                            Layout.fillWidth: true
                            Layout.preferredHeight: plainBody.implicitHeight + theme.cardPadding * 2
                            color: Qt.rgba(0.89, 0, 0.06, 0.06)
                            border.color: Design.accent(theme)
                            border.width: 1

                            Rectangle {
                                anchors.left: parent.left
                                anchors.top: parent.top
                                anchors.bottom: parent.bottom
                                width: 4
                                color: Design.accent(theme)
                            }

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: theme.cardPadding + 8
                                anchors.rightMargin: theme.cardPadding + 2
                                anchors.topMargin: theme.cardPadding
                                anchors.bottomMargin: theme.cardPadding
                                spacing: theme.spacingSm + 2

                                Text {
                                    text: "\u26a0 INSECURE"
                                    color: Design.accent(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontBodySmall
                                    font.weight: Font.Bold
                                    font.letterSpacing: 1.4
                                }

                                Text {
                                    id: plainBody
                                    Layout.fillWidth: true
                                    text: "<b>Do not use this for real books.</b> Plain mode is intended for debugging and early evaluation only \u2014 your database is readable by anything with disk access. Switch to encrypted before tracking real funds via <b>Settings \u2192 App lock</b>."
                                    textFormat: Text.StyledText
                                    color: Design.ink2(theme)
                                    font.family: Design.sans()
                                    font.pixelSize: theme.fontBody
                                    lineHeight: 1.55
                                    wrapMode: Text.WordWrap
                                }
                            }
                        }

                        Item { Layout.fillHeight: true }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 1
                            color: Design.ink(theme)
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            Layout.topMargin: 6

                            Button {
                                flat: true
                                padding: 0
                                onClicked: root.step = "setup"
                                implicitHeight: 24

                                contentItem: Text {
                                    text: "\u2190 BACK"
                                    color: Design.ink2(theme)
                                    font.family: Design.mono(theme)
                                    font.pixelSize: theme.fontBodySmall
                                    font.letterSpacing: 1.4
                                }

                                background: Rectangle { color: "transparent" }
                            }

                            Item { Layout.fillWidth: true }

                            ActionButton {
                                variant: "primary"
                                size: "lg"
                                text: "\u2192  Open ledger"
                                enabled: root.canFinish
                                onClicked: dashboardVM.selectPage("overview")
                            }
                        }
                    }
                }
            }
        }
    }
}
