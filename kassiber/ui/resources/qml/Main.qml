import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "components"
import "dialogs"

ApplicationWindow {
    id: root
    objectName: "mainWindow"
    visible: false
    title: dashboardVM.windowTitle
    width: windowState.width > 0 ? windowState.width : 1240
    height: windowState.height > 0 ? windowState.height : 820
    minimumWidth: 980
    minimumHeight: 640
    color: theme.bg

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
        anchors.margins: theme.spacingLg
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

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            Rectangle {
                anchors.centerIn: parent
                width: Math.min(parent.width - theme.spacingXl, 680)
                height: 360
                color: theme.card
                radius: theme.radiusLg
                border.color: theme.cardBorder
                border.width: 1

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: theme.spacingXl
                    spacing: theme.spacingMd

                    Rectangle {
                        Layout.alignment: Qt.AlignHCenter
                        width: 64
                        height: 64
                        radius: 32
                        color: "transparent"
                        border.color: theme.chipBorder
                        border.width: 1

                        Text {
                            anchors.centerIn: parent
                            text: connectionsVM.isEmpty ? "+" : "i"
                            color: theme.accent
                            font.family: theme.displayFont
                            font.pixelSize: 28
                            font.bold: true
                        }
                    }

                    Text {
                        Layout.alignment: Qt.AlignHCenter
                        text: dashboardVM.shellTitle
                        color: theme.ink
                        font.family: theme.displayFont
                        font.pixelSize: 28
                        font.bold: true
                    }

                    Text {
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.WordWrap
                        text: dashboardVM.shellBody
                        color: theme.inkMuted
                        font.family: theme.bodyFont
                        font.pixelSize: 15
                    }

                    PrimaryButton {
                        Layout.alignment: Qt.AlignHCenter
                        text: connectionsVM.ctaLabel
                        enabled: connectionsVM.canOpenAddConnection
                        onClicked: addConnectionDialog.open()
                    }

                    Text {
                        Layout.alignment: Qt.AlignHCenter
                        text: connectionsVM.emptyBadge + "  |  " + connectionsVM.connectionCount + " wallet(s)"
                        color: theme.inkMuted
                        font.family: theme.bodyFont
                        font.pixelSize: 12
                    }

                    Repeater {
                        model: dashboardVM.notices
                        delegate: Rectangle {
                            Layout.fillWidth: true
                            radius: theme.radiusMd
                            color: "#FAF7F0"
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
                onClicked: settingsDialog.open()
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
