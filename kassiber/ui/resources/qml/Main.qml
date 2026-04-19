import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "components"
import "dialogs"
import "pages"

ApplicationWindow {
    id: root
    objectName: "mainWindow"
    readonly property bool previewSceneMode: (uiPreviewPage || "") !== ""
    readonly property bool captureMode: !!uiCaptureMode
    readonly property bool standaloneWelcomeMode: dashboardVM.currentPage === "welcome" && (!dashboardVM.hasProfile || previewSceneMode)
    property bool hideSensitivePreview: false
    property string previewLanguage: "EN"

    visible: false
    title: dashboardVM.windowTitle
    width: windowState.width > 0 ? windowState.width : 1240
    height: windowState.height > 0 ? windowState.height : 820
    minimumWidth: 1060
    minimumHeight: 700
    color: standaloneWelcomeMode ? theme.warmBg : theme.bg

    AddConnectionDialog {
        id: addConnectionDialog
    }

    SettingsDialog {
        id: settingsDialog
    }

    Popup {
        id: projectPopup
        width: 360
        x: root.width - width - 18
        y: 52
        padding: 0
        modal: false
        focus: true
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        background: Rectangle {
            color: theme.paper
            border.color: theme.ink
            border.width: 1
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            Rectangle {
                Layout.fillWidth: true
                height: 40
                color: "transparent"
                border.color: theme.line
                border.width: 0

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: theme.line
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 14
                    anchors.rightMargin: 14

                    Text {
                        text: "Switch profile"
                        color: theme.ink3
                        font.family: theme.monoFont
                        font.pixelSize: 9
                        font.bold: true
                        font.letterSpacing: 1.4
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: "MANAGE \u2192"
                        color: theme.accent
                        font.family: theme.monoFont
                        font.pixelSize: 9
                        font.bold: true
                        font.letterSpacing: 1.4
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                Repeater {
                    model: dashboardVM.availableProfiles

                    Button {
                        Layout.fillWidth: true
                        implicitHeight: 56
                        flat: true

                        contentItem: RowLayout {
                            spacing: 10

                            Rectangle {
                                width: 8
                                height: 8
                                radius: 4
                                color: modelData["current"] === "yes" ? theme.accent : "transparent"
                                border.color: modelData["current"] === "yes" ? theme.accent : theme.line
                                border.width: 1
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2

                                Text {
                                    Layout.fillWidth: true
                                    text: modelData["label"] || ""
                                    color: theme.ink
                                    font.family: theme.serifFont
                                    font.pixelSize: 15
                                    elide: Text.ElideRight
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: (modelData["tax_country"] || "").toUpperCase() + "  \u00b7  " + (modelData["gains_algorithm"] || "")
                                    color: theme.ink3
                                    font.family: theme.monoFont
                                    font.pixelSize: 10
                                    elide: Text.ElideRight
                                }
                            }
                        }

                        background: Rectangle {
                            color: modelData["current"] === "yes" ? theme.paper2 : "transparent"
                            border.color: theme.line
                            border.width: 0

                            Rectangle {
                                visible: index > 0
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                height: 1
                                color: theme.line
                            }
                        }
                    }
                }
            }
        }
    }

    Rectangle {
        visible: previewSceneMode && !captureMode
        z: 20
        x: 12
        y: 12
        color: theme.paper2
        border.color: theme.ink
        border.width: 1

        RowLayout {
            anchors.fill: parent
            anchors.margins: 6
            spacing: 4

            Text {
                text: "SCENE"
                color: theme.ink3
                font.family: theme.monoFont
                font.pixelSize: 9
                font.letterSpacing: 1.0
            }

            Repeater {
                model: [
                    { "id": "welcome", "label": "Welcome" },
                    { "id": "overview-empty", "label": "Overview \u00b7 empty" },
                    { "id": "overview-data", "label": "Overview \u00b7 data" },
                    { "id": "transactions", "label": "Transactions" },
                    { "id": "tax", "label": "Tax \u00b7 capital gains" },
                    { "id": "connection-detail", "label": "Connection detail" }
                ]

                Button {
                    flat: true
                    leftPadding: 8
                    rightPadding: 8
                    topPadding: 3
                    bottomPadding: 3
                    onClicked: dashboardVM.selectPage(modelData["id"])

                    contentItem: Text {
                        text: modelData["label"]
                        color: {
                            var current = dashboardVM.currentPage
                            var wanted = modelData["id"]
                            var active = current === wanted
                            if ((wanted === "overview-empty" || wanted === "overview-data") && current === "overview") {
                                active = true
                            }
                            if (wanted === "tax" && current === "reports") {
                                active = true
                            }
                            return active ? theme.paper : theme.ink
                        }
                        font.family: theme.monoFont
                        font.pixelSize: 10
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }

                    background: Rectangle {
                        color: {
                            var current = dashboardVM.currentPage
                            var wanted = modelData["id"]
                            var active = current === wanted
                            if ((wanted === "overview-empty" || wanted === "overview-data") && current === "overview") {
                                active = true
                            }
                            if (wanted === "tax" && current === "reports") {
                                active = true
                            }
                            return active ? theme.ink : "transparent"
                        }
                    }
                }
            }
        }
    }

    WelcomePage {
        anchors.fill: parent
        visible: root.standaloneWelcomeMode
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        visible: !root.standaloneWelcomeMode

        Rectangle {
            Layout.fillWidth: true
            height: 54
            color: theme.paper

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: theme.ink
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 18
                anchors.rightMargin: 18
                spacing: 18

                Wordmark {
                    size: 20
                    inkColor: theme.ink
                }

                Rectangle {
                    width: 1
                    Layout.fillHeight: true
                    Layout.topMargin: 16
                    Layout.bottomMargin: 16
                    color: theme.line
                }

                Repeater {
                    model: [
                        { "id": "overview", "label": "Overview" },
                        { "id": "transactions", "label": "Transactions" },
                        { "id": "reports", "label": "Reports" }
                    ]

                    Button {
                        visible: true
                        enabled: true
                        flat: true
                        padding: 0
                        onClicked: dashboardVM.selectPage(modelData["id"])

                        contentItem: Text {
                            text: modelData["label"]
                            color: dashboardVM.currentPage === modelData["id"] ? theme.ink : theme.ink3
                            font.family: theme.sansFont
                            font.pixelSize: 12
                            font.bold: dashboardVM.currentPage === modelData["id"]
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }

                        background: Item {
                            implicitWidth: 110
                            implicitHeight: 30

                            Rectangle {
                                visible: dashboardVM.currentPage === modelData["id"]
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                anchors.leftMargin: 10
                                anchors.rightMargin: 10
                                height: 2
                                color: theme.accent
                            }
                        }
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                Button {
                    text: dashboardVM.projectSummary
                    flat: true
                    padding: 0
                    onClicked: projectPopup.open()

                    contentItem: Text {
                        text: parent.text
                        color: theme.ink2
                        font.family: theme.monoFont
                        font.pixelSize: 11
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        elide: Text.ElideRight
                    }

                    background: Rectangle {
                        implicitWidth: 248
                        implicitHeight: 34
                        color: theme.paper3
                        radius: 17
                        border.color: theme.line
                        border.width: 1
                    }
                }

                Rectangle {
                    implicitWidth: 58
                    implicitHeight: 26
                    color: "transparent"
                    border.color: theme.line
                    border.width: 1

                    RowLayout {
                        anchors.fill: parent
                        spacing: 0

                        Repeater {
                            model: ["EN", "DE"]

                            Button {
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                flat: true
                                padding: 0
                                onClicked: root.previewLanguage = modelData

                                contentItem: Text {
                                    text: modelData
                                    color: root.previewLanguage === modelData ? theme.paper : theme.ink2
                                    font.family: theme.monoFont
                                    font.pixelSize: 10
                                    font.bold: true
                                    horizontalAlignment: Text.AlignHCenter
                                    verticalAlignment: Text.AlignVCenter
                                }

                                background: Rectangle {
                                    color: root.previewLanguage === modelData ? theme.ink : "transparent"
                                }
                            }
                        }
                    }
                }

                ToolButton {
                    text: root.hideSensitivePreview ? "\u25c9" : "\u25ce"
                    onClicked: root.hideSensitivePreview = !root.hideSensitivePreview
                    font.family: theme.monoFont
                    font.pixelSize: 12
                    palette.buttonText: root.hideSensitivePreview ? theme.paper : theme.ink2
                    background: Rectangle {
                        color: root.hideSensitivePreview ? theme.ink : "transparent"
                        border.color: root.hideSensitivePreview ? theme.ink : theme.line
                        border.width: 1
                        implicitWidth: 26
                        implicitHeight: 26
                    }
                }

                ToolButton {
                    text: "\ud83d\udd12"
                    font.pixelSize: 11
                    palette.buttonText: theme.ink2
                    background: Rectangle {
                        color: "transparent"
                        border.color: theme.line
                        border.width: 1
                        implicitWidth: 26
                        implicitHeight: 26
                    }
                }

                ToolButton {
                    text: "\u2699"
                    onClicked: settingsDialog.open()
                    font.family: theme.monoFont
                    font.pixelSize: 13
                    palette.buttonText: theme.ink2
                    background: Rectangle {
                        color: "transparent"
                        border.color: theme.line
                        border.width: 1
                        implicitWidth: 28
                        implicitHeight: 28
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: theme.paper

            StackLayout {
                anchors.fill: parent
                anchors.margins: dashboardVM.currentPage === "welcome" ? 0 : 18
                currentIndex: dashboardVM.pageIndex

                WelcomePage {
                }

                OverviewPage {
                    onRequestAddConnection: addConnectionDialog.open()
                }

                ConnectionDetailPage {
                    onRequestAddConnection: addConnectionDialog.open()
                }

                TransactionsPage {
                }

                ReportsPage {
                }

                SettingsPage {
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            height: 28
            color: theme.paper

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: 1
                color: theme.line
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 18
                anchors.rightMargin: 18
                spacing: 18

                Text {
                    text: "KASSIBER " + settingsVM.versionText
                    color: theme.ink3
                    font.family: theme.monoFont
                    font.pixelSize: 10
                }

                Text {
                    text: "\u25cf"
                    color: theme.ok
                    font.family: theme.monoFont
                    font.pixelSize: 10
                }

                Text {
                    text: "LOCAL"
                    color: theme.ink3
                    font.family: theme.monoFont
                    font.pixelSize: 10
                }

                Text {
                    Layout.fillWidth: true
                    text: dashboardVM.footerSummary
                    color: theme.ink3
                    font.family: theme.monoFont
                    font.pixelSize: 10
                    elide: Text.ElideMiddle
                }

                Rectangle {
                    color: theme.paper2
                    border.color: theme.line
                    border.width: 1
                    implicitWidth: donateLabel.implicitWidth + 28
                    implicitHeight: 28

                    Text {
                        id: donateLabel
                        anchors.centerIn: parent
                        text: "DONATE SATS"
                        color: theme.accent
                        font.family: theme.monoFont
                        font.pixelSize: 10
                        font.letterSpacing: 1.0
                    }
                }

                Text {
                    text: "BTC/EUR \u00b7 COINGECKO"
                    color: theme.ink3
                    font.family: theme.monoFont
                    font.pixelSize: 10
                }

                ToolButton {
                    text: "GITHUB"
                    font.family: theme.monoFont
                    font.pixelSize: 10
                }

                ToolButton {
                    text: "SETTINGS"
                    onClicked: dashboardVM.selectPage("settings")
                    font.family: theme.monoFont
                    font.pixelSize: 10
                }
            }
        }
    }
}
