import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

Dialog {
    id: root
    title: "Add Connection"
    modal: true
    width: 460
    standardButtons: Dialog.Close

    contentItem: ColumnLayout {
        spacing: theme.spacingMd

        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Phase 1 ships a placeholder shell only. Real connection setup lands in Phase 3."
            color: theme.ink
            font.family: theme.displayFont
            font.pixelSize: 20
        }

        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Use today's CLI commands to create wallets, import data, and sync connections. Once those flows move into the UI, this dialog will host them."
            color: theme.inkMuted
            font.family: theme.bodyFont
            font.pixelSize: 13
        }
    }
}
