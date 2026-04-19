import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "components"
import "dialogs"
import "pages"

ApplicationWindow {
    id: root
    objectName: "mainWindow"
    readonly property bool welcomeMode: dashboardVM.currentPage === "welcome"
    visible: false
    title: dashboardVM.windowTitle
    width: windowState.width > 0 ? windowState.width : 1240
    height: windowState.height > 0 ? windowState.height : 820
    minimumWidth: 980
    minimumHeight: 640
    color: welcomeMode ? theme.warmBg : theme.bg

    AddConnectionDialog {
        id: addConnectionDialog
    }

    SettingsDialog {
        id: settingsDialog
    }

    Popup {
        id: projectPopup
        width: 300
        x: root.width - width - theme.spacingLg
        y: theme.spacingLg + 56
        padding: theme.spacingMd
        modal: false
        focus: true
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        background: Rectangle {
            color: theme.card
            radius: theme.radiusLg
            border.color: theme.cardBorder
            border.width: 1
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: theme.spacingSm

            Text {
                text: "Current project"
                color: theme.ink
                font.family: theme.displayFont
                font.pixelSize: 18
            }

            Text {
                text: dashboardVM.projectLabel
                color: theme.inkMuted
                font.family: theme.bodyFont
                font.pixelSize: 13
            }

            Repeater {
                model: dashboardVM.availableProfiles
                delegate: Rectangle {
                    Layout.fillWidth: true
                    height: 40
                    radius: theme.radiusMd
                    color: "transparent"
                    border.color: theme.cardBorder
                    border.width: 1

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: theme.spacingMd
                        anchors.rightMargin: theme.spacingMd
                        verticalAlignment: Text.AlignVCenter
                        text: modelData["label"] + (modelData["current"] === "yes" ? " (current)" : "")
                        color: theme.ink
                        font.family: theme.bodyFont
                        font.pixelSize: 13
                        elide: Text.ElideRight
                    }
                }
            }

            Text {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Profile switching stays read-only in Phase 1. Editing and management land later."
                color: theme.inkMuted
                font.family: theme.bodyFont
                font.pixelSize: 12
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: welcomeMode ? 0 : theme.spacingLg
        spacing: welcomeMode ? 0 : theme.spacingLg

        WelcomePage {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: root.welcomeMode
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: !root.welcomeMode
            spacing: theme.spacingLg

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.spacingMd

                Rectangle {
                    width: 42
                    height: 42
                    radius: 21
                    color: theme.accent

                    Text {
                        anchors.centerIn: parent
                        text: "K"
                        color: "#FFFFFF"
                        font.family: theme.displayFont
                        font.pixelSize: 22
                        font.bold: true
                    }
                }

                ColumnLayout {
                    spacing: 2

                    Text {
                        text: "Kassiber"
                        color: theme.ink
                        font.family: theme.displayFont
                        font.pixelSize: 26
                        font.bold: true
                    }

                    Text {
                        text: dashboardVM.phaseSummary
                        color: theme.inkMuted
                        font.family: theme.bodyFont
                        font.pixelSize: 13
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                Button {
                    text: dashboardVM.projectLabel
                    onClicked: projectPopup.open()
                    contentItem: Text {
                        text: parent.text
                        color: theme.ink
                        font.family: theme.bodyFont
                        font.pixelSize: 13
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        elide: Text.ElideRight
                    }
                    background: Rectangle {
                        color: theme.card
                        radius: theme.radiusLg
                        border.color: theme.chipBorder
                        border.width: 1
                    }
                    implicitWidth: 240
                    implicitHeight: 42
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: theme.spacingLg

                Card {
                    Layout.preferredWidth: 280
                    Layout.fillHeight: true

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: theme.spacingMd

                        Text {
                            text: "Screens"
                            color: theme.ink
                            font.family: theme.displayFont
                            font.pixelSize: 24
                            font.bold: true
                        }

                        Text {
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                            text: "This left rail mirrors the mockup list so each screen can be translated into QML incrementally."
                            color: theme.inkMuted
                            font.family: theme.bodyFont
                            font.pixelSize: 13
                        }

                        Repeater {
                            model: dashboardVM.pages

                            Button {
                                Layout.fillWidth: true
                                enabled: modelData["enabled"] === true
                                onClicked: dashboardVM.selectPage(modelData["id"])

                                contentItem: Column {
                                    Text {
                                        text: modelData["label"]
                                        color: enabled ? theme.ink : theme.inkMuted
                                        font.family: theme.displayFont
                                        font.pixelSize: 18
                                        font.bold: true
                                    }

                                    Text {
                                        text: modelData["caption"]
                                        color: theme.inkMuted
                                        font.family: theme.bodyFont
                                        font.pixelSize: 12
                                    }
                                }

                                background: Rectangle {
                                    color: dashboardVM.currentPage === modelData["id"] ? theme.cardAlt : "transparent"
                                    radius: theme.radiusMd
                                    border.color: theme.cardBorder
                                    border.width: 1
                                }

                                implicitHeight: 72
                            }
                        }

                        Item {
                            Layout.fillHeight: true
                        }

                        Repeater {
                            model: dashboardVM.notices
                            delegate: Rectangle {
                                Layout.fillWidth: true
                                radius: theme.radiusMd
                                color: theme.cardAlt
                                border.color: theme.cardBorder
                                border.width: 1
                                implicitHeight: noticeText.implicitHeight + theme.spacingMd

                                Text {
                                    id: noticeText
                                    anchors.fill: parent
                                    anchors.margins: theme.spacingSm
                                    wrapMode: Text.WordWrap
                                    text: modelData
                                    color: theme.inkMuted
                                    font.family: theme.bodyFont
                                    font.pixelSize: 12
                                }
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    StackLayout {
                        anchors.fill: parent
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
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: theme.spacingMd

                Text {
                    text: settingsVM.versionText
                    color: theme.inkMuted
                    font.family: theme.bodyFont
                    font.pixelSize: 12
                }

                Text {
                    text: dashboardVM.footerSummary
                    color: theme.inkMuted
                    font.family: theme.bodyFont
                    font.pixelSize: 12
                }

                Item {
                    Layout.fillWidth: true
                }

                ToolButton {
                    text: "Settings"
                    onClicked: dashboardVM.selectPage("settings")
                }

                ToolButton {
                    text: "GitHub"
                }

                ToolButton {
                    text: "Nostr"
                }

                ToolButton {
                    text: "Support the App"
                }
            }
        }
    }
}
