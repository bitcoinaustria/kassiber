import SwiftUI
import Charts
import KassiberDaemonKit
import KassiberViewModels

private func dashboardChartLocalized(_ key: String) -> String { AppLocalization.string(key) }

/// Native counterpart of Tauri's treasury chart. The chart deliberately uses
/// the daemon's dated portfolio series and activity rows directly: no random
/// interpolation or client-side balance invention is allowed.
struct DashboardActivityChart: View {
    let points: [DashboardPoint]
    let transactions: [TransactionRow]
    let fiatCurrency: String
    let marketRate: Double?
    let onOpenTransaction: (TransactionRow) -> Void
    var expanded = false

    @State private var period: DashboardChartPeriod = .automatic
    @State private var visibleSeries: Set<DashboardChartSeries> = Set(DashboardChartSeries.allCases)
    @State private var logScale = false
    @State private var autoFit = true
    @State private var showLastValue = true
    @State private var groupEvents = true
    @State private var selectedDate: Date?
    @State private var showExpanded = false
    @State private var brushStart = 0.0
    @State private var brushEnd = 1.0
    @State private var incomingMarkerMinimumBTC = 0.0025
    @State private var outgoingMarkerMinimumBTC = 0.0

    private var latestDate: Date { points.last?.date ?? Date() }

    private var visiblePoints: [DashboardPoint] {
        let cutoff = dashboardChartCutoff(period, latest: latestDate)
        let periodPoints = points.filter { point in
            guard let cutoff else { return true }
            return point.date >= cutoff
        }
        guard periodPoints.count > 3 else { return periodPoints }
        let start = min(max(0, Int((Double(periodPoints.count - 1) * brushStart).rounded())), periodPoints.count - 1)
        let end = max(start + 1, min(periodPoints.count - 1, Int((Double(periodPoints.count - 1) * brushEnd).rounded())))
        return Array(periodPoints[start...end])
    }

    private var visibleEvents: [TransactionRow] {
        let cutoff = dashboardChartCutoff(period, latest: latestDate)
        let filtered = transactions.filter { row in
            guard !row.excluded, let date = row.occurredAt else { return false }
            guard let cutoff else { return true }
            return date >= cutoff
        }.filter { row in
            let amountBTC = abs(Double(row.amountSats)) / 100_000_000
            if row.amountSats >= 0 { return amountBTC >= incomingMarkerMinimumBTC }
            return amountBTC >= outgoingMarkerMinimumBTC
        }
        guard groupEvents else { return filtered }
        // Keep the largest event in each day and expose the count in the
        // annotation. This mirrors the Tauri marker grouping without hiding
        // the underlying rows from the detail table.
        var grouped: [String: TransactionRow] = [:]
        for row in filtered {
            let key = row.occurredAt.map { dayKey($0) } ?? row.id
            if let current = grouped[key], abs(current.amountSats) >= abs(row.amountSats) { continue }
            grouped[key] = row
        }
        return grouped.values.sorted { ($0.occurredAt ?? .distantPast) < ($1.occurredAt ?? .distantPast) }
    }

    private var balanceValues: [Double] {
        var values = visibleSeries.contains(.balance) ? visiblePoints.map { $0.balanceBTC } : []
        if visibleSeries.contains(.events) {
            values += visibleEvents.map { eventBalance($0) }
        }
        return values.filter { $0.isFinite && $0 >= 0 }
    }

    private var fiatValues: [Double] {
        guard !fiatCurrency.isEmpty else { return [] }
        return visiblePoints.flatMap { point -> [Double] in
            var values: [Double] = []
            if visibleSeries.contains(.portfolioValue) { values.append(point.fiatValue) }
            if visibleSeries.contains(.costBasis) { values.append(point.costBasisEUR) }
            if visibleSeries.contains(.price), let price = point.priceEUR { values.append(price) }
            return values
        }.filter { $0.isFinite && $0 >= 0 }
    }

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                chartHeader
                if visiblePoints.isEmpty {
                    ContentUnavailableView(
                        dashboardChartLocalized("dashboard.chart.empty"),
                        systemImage: "chart.xyaxis.line",
                        description: Text(dashboardChartLocalized("dashboard.chart.emptyHint"))
                    )
                    .frame(height: expanded ? 520 : 250)
                } else {
                    chart
                        .frame(height: expanded ? 500 : 238)
                }
                chartControls
            }
            .padding(10)
        } label: {
            Label(dashboardChartLocalized("dashboard.chart.title"), systemImage: "chart.xyaxis.line")
        }
        .sheet(isPresented: $showExpanded) {
            DashboardActivityChart(
                points: points,
                transactions: transactions,
                fiatCurrency: fiatCurrency,
                marketRate: marketRate,
                onOpenTransaction: onOpenTransaction,
                expanded: true
            )
            .padding(24)
            .frame(minWidth: 980, minHeight: 680)
        }
    }

    private var chartHeader: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 2) {
                Text(dashboardChartLocalized("dashboard.chart.subtitle"))
                    .font(.caption).foregroundStyle(.secondary)
                if showLastValue, let latest = visiblePoints.last {
                    Text(lastValueLabel(latest))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .kassiberSensitive()
                }
            }
            Spacer()
            if !expanded {
                Button {
                    showExpanded = true
                } label: {
                    Label(dashboardChartLocalized("dashboard.chart.expand"), systemImage: "arrow.up.left.and.arrow.down.right")
                }
                .buttonStyle(.borderless)
            }
            if let event = selectedEvent {
                Button {
                    onOpenTransaction(event)
                } label: {
                    Label(dashboardChartLocalized("dashboard.chart.openTransaction"), systemImage: "arrow.up.right.square")
                }
                .buttonStyle(.borderless)
            }
        }
    }

    private var chart: some View {
        ZStack {
            Chart {
                if visibleSeries.contains(.balance) {
                    ForEach(visiblePoints) { point in
                        AreaMark(
                            x: .value(dashboardChartLocalized("field.date"), point.date),
                            y: .value(dashboardChartLocalized("field.balance"), chartNumber(point.balanceBTC))
                        )
                        .foregroundStyle(by: .value("Series", "Balance"))
                        .opacity(0.16)
                        LineMark(
                            x: .value(dashboardChartLocalized("field.date"), point.date),
                            y: .value(dashboardChartLocalized("field.balance"), chartNumber(point.balanceBTC))
                        )
                        .foregroundStyle(by: .value("Series", "Balance"))
                        .lineStyle(StrokeStyle(lineWidth: 1.75))
                    }
                }
                if visibleSeries.contains(.events) {
                    ForEach(visibleEvents) { event in
                        if let date = event.occurredAt {
                            PointMark(
                                x: .value(dashboardChartLocalized("field.date"), date),
                                y: .value(dashboardChartLocalized("dashboard.chart.events"), chartNumber(eventBalance(event)))
                            )
                            .foregroundStyle(by: .value(
                                "Series",
                                event.amountSats >= 0 ? "Incoming" : "Outgoing"
                            ))
                            .symbolSize(groupEvents ? 60 : 36)
                        }
                    }
                }
                if let selectedDate {
                    RuleMark(x: .value(dashboardChartLocalized("field.date"), selectedDate))
                        .foregroundStyle(.secondary.opacity(0.5))
                        .annotation(position: .top, alignment: .leading) {
                            Text(selectedDate, format: .dateTime.year().month().day())
                                .font(.caption2).padding(4).background(.thinMaterial, in: RoundedRectangle(cornerRadius: 4))
                        }
                }
            }
            .chartYScale(domain: balanceDomain)
            .chartXAxis { AxisMarks(values: .automatic(desiredCount: expanded ? 8 : 5)) }
            .chartYAxis { AxisMarks(position: .leading) }
            .chartForegroundStyleScale([
                "Balance": Color.kassiberAccent,
                "Incoming": Color.green,
                "Outgoing": Color.red,
            ])
            .chartXSelection(value: $selectedDate)
            .onChange(of: selectedDate) { _, date in
                guard let date, let event = nearestEvent(to: date) else { return }
                selectedEvent = event
            }

            if !fiatValues.isEmpty {
                Chart {
                    if visibleSeries.contains(.portfolioValue) {
                        ForEach(visiblePoints) { point in
                            LineMark(
                                x: .value(dashboardChartLocalized("field.date"), point.date),
                                y: .value(dashboardChartLocalized("dashboard.marketValue"), chartNumber(point.fiatValue))
                            )
                            .foregroundStyle(by: .value("Series", "Market Value"))
                        }
                    }
                    if visibleSeries.contains(.costBasis) {
                        ForEach(visiblePoints) { point in
                            LineMark(
                                x: .value(dashboardChartLocalized("field.date"), point.date),
                                y: .value(dashboardChartLocalized("dashboard.costBasis"), chartNumber(point.costBasisEUR))
                            )
                            .foregroundStyle(by: .value("Series", "Cost Basis"))
                            .lineStyle(StrokeStyle(dash: [5, 3]))
                        }
                    }
                    if visibleSeries.contains(.price) {
                        ForEach(visiblePoints) { point in
                            if let price = point.priceEUR {
                                LineMark(
                                    x: .value(dashboardChartLocalized("field.date"), point.date),
                                    y: .value(dashboardChartLocalized("dashboard.chart.price"), chartNumber(price))
                                )
                                .foregroundStyle(by: .value("Series", "BTC price"))
                                .lineStyle(StrokeStyle(dash: [3, 5]))
                            }
                        }
                    }
                }
                .chartYScale(domain: fiatDomain)
                .chartXAxis(.hidden)
                .chartYAxis { AxisMarks(position: .trailing) }
                .chartForegroundStyleScale([
                    "Market Value": Color.orange,
                    "Cost Basis": Color.purple,
                    "BTC price": Color.pink,
                ])
                .allowsHitTesting(false)
            }
        }
        .chartLegend(.hidden)
        .chartPlotStyle { plot in
            plot
                .background(Color.kassiberCanvas.opacity(0.18))
                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
        .kassiberSensitive()
    }

    private var chartControls: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Picker(dashboardChartLocalized("dashboard.chart.period"), selection: $period) {
                    Text(dashboardChartLocalized("period.auto")).tag(DashboardChartPeriod.automatic)
                    Text(dashboardChartLocalized("period.30days")).tag(DashboardChartPeriod.days30)
                    Text(dashboardChartLocalized("period.3months")).tag(DashboardChartPeriod.months3)
                    Text(dashboardChartLocalized("period.6months")).tag(DashboardChartPeriod.months6)
                    Text(dashboardChartLocalized("period.ytd")).tag(DashboardChartPeriod.ytd)
                    Text(dashboardChartLocalized("period.1year")).tag(DashboardChartPeriod.year1)
                    Text(dashboardChartLocalized("period.5years")).tag(DashboardChartPeriod.years5)
                    Text(dashboardChartLocalized("period.all")).tag(DashboardChartPeriod.all)
                }
                .pickerStyle(.segmented)
                Spacer()
                Toggle(dashboardChartLocalized("dashboard.chart.log"), isOn: $logScale)
                Toggle(dashboardChartLocalized("dashboard.chart.autoFit"), isOn: $autoFit)
                Toggle(dashboardChartLocalized("dashboard.chart.lastValue"), isOn: $showLastValue)
                Toggle(dashboardChartLocalized("dashboard.chart.groupEvents"), isOn: $groupEvents)
            }
            HStack {
                Text(dashboardChartLocalized("dashboard.chart.series")).font(.caption).foregroundStyle(.secondary)
                ForEach(DashboardChartSeries.allCases) { series in
                    Toggle(seriesLabel(series), isOn: Binding(
                        get: { visibleSeries.contains(series) },
                        set: { isVisible in
                            if isVisible { visibleSeries.insert(series) } else { visibleSeries.remove(series) }
                        }
                    ))
                    .toggleStyle(.checkbox)
                }
            }
            if points.count > 3 {
                HStack(spacing: 8) {
                    Text(dashboardChartLocalized("dashboard.chart.window")).font(.caption).foregroundStyle(.secondary)
                    Slider(value: $brushStart, in: 0...max(0.01, brushEnd - 0.01), step: 0.01)
                    Slider(value: $brushEnd, in: min(0.99, brushStart + 0.01)...1, step: 0.01)
                    Button(dashboardChartLocalized("dashboard.chart.resetWindow")) {
                        brushStart = 0; brushEnd = 1
                    }
                    .buttonStyle(.borderless)
                }
            }
            if visibleSeries.contains(.events) {
                HStack(spacing: 8) {
                    Text(dashboardChartLocalized("dashboard.chart.incomingMinimum")).font(.caption).foregroundStyle(.secondary)
                    Slider(value: $incomingMarkerMinimumBTC, in: 0...0.05, step: 0.0005)
                    Text(KassiberFormatting.btc(incomingMarkerMinimumBTC, locale: .current)).font(.caption2.monospacedDigit())
                    Text(dashboardChartLocalized("dashboard.chart.outgoingMinimum")).font(.caption).foregroundStyle(.secondary)
                    Slider(value: $outgoingMarkerMinimumBTC, in: 0...0.05, step: 0.0005)
                    Text(KassiberFormatting.btc(outgoingMarkerMinimumBTC, locale: .current)).font(.caption2.monospacedDigit())
                }
            }
        }
        .controlSize(.small)
        .font(.caption)
    }

    @State private var selectedEvent: TransactionRow?

    private var balanceDomain: ClosedRange<Double> {
        domain(for: balanceValues)
    }

    private var fiatDomain: ClosedRange<Double> {
        domain(for: fiatValues)
    }

    private func domain(for values: [Double]) -> ClosedRange<Double> {
        let values = values.map(chartNumber)
        guard !values.isEmpty else { return 0...1 }
        let maximum = values.max() ?? 1
        let minimum = values.min() ?? 0
        if logScale { return minimum...(max(minimum + 1, maximum * 1.12)) }
        if autoFit { return min(0, minimum)...max(1, maximum * 1.12) }
        return 0...max(1, maximum * 1.25)
    }

    private func chartNumber(_ value: Double) -> Double {
        guard logScale else { return value }
        return value > 0 ? log10(value) : 0
    }

    private func eventBalance(_ row: TransactionRow) -> Double {
        guard let date = row.occurredAt else { return visiblePoints.last?.balanceBTC ?? 0 }
        return visiblePoints.min {
            abs($0.date.timeIntervalSince(date)) < abs($1.date.timeIntervalSince(date))
        }?.balanceBTC ?? visiblePoints.last?.balanceBTC ?? 0
    }

    private func nearestEvent(to date: Date) -> TransactionRow? {
        visibleEvents.min { lhs, rhs in
            abs((lhs.occurredAt ?? .distantPast).timeIntervalSince(date)) < abs((rhs.occurredAt ?? .distantPast).timeIntervalSince(date))
        }
    }

    private func lastValueLabel(_ point: DashboardPoint) -> String {
        let value = visibleSeries.contains(.portfolioValue) ? point.fiatValue : point.balanceBTC
        return visibleSeries.contains(.portfolioValue)
            ? KassiberFormatting.fiat(value, currency: fiatCurrency, locale: .current)
            : KassiberFormatting.btc(value, locale: .current)
    }

    private func seriesLabel(_ series: DashboardChartSeries) -> String {
        switch series {
        case .balance: dashboardChartLocalized("field.balance")
        case .portfolioValue: dashboardChartLocalized("dashboard.marketValue")
        case .costBasis: dashboardChartLocalized("dashboard.costBasis")
        case .price: dashboardChartLocalized("dashboard.chart.price")
        case .events: dashboardChartLocalized("dashboard.chart.events")
        }
    }
}

private func dashboardChartCutoff(_ period: DashboardChartPeriod, latest: Date) -> Date? {
    switch period {
    case .automatic, .all: return nil
    case .days30: return latest.addingTimeInterval(-30 * 86_400)
    case .months3: return latest.addingTimeInterval(-90 * 86_400)
    case .months6: return latest.addingTimeInterval(-180 * 86_400)
    case .ytd:
        return Calendar.current.date(from: Calendar.current.dateComponents([.year], from: latest))
    case .year1: return latest.addingTimeInterval(-365 * 86_400)
    case .years5: return latest.addingTimeInterval(-5 * 365 * 86_400)
    }
}

private func dayKey(_ date: Date) -> String {
    let components = Calendar.current.dateComponents([.year, .month, .day], from: date)
    return "\(components.year ?? 0)-\(components.month ?? 0)-\(components.day ?? 0)"
}

struct WalletBalanceHistoryChart: View {
    let points: [WalletHistoryPoint]
    let currency: KassiberDisplayCurrency
    let priceEUR: Double?

    @State private var selectedDate: Date?

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(points.count > 1 ? changeLabel : dashboardChartLocalized("dashboard.chart.empty"))
                        .font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    if let selectedDate, let point = nearest(to: selectedDate) {
                        Text(format(point.amountSats))
                            .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                    }
                }
                Chart(points) { point in
                    AreaMark(
                        x: .value(dashboardChartLocalized("field.date"), point.date),
                        y: .value(dashboardChartLocalized("field.balance"), Double(point.amountSats) / 100_000_000)
                    )
                    .foregroundStyle(.tint.opacity(0.14))
                    LineMark(
                        x: .value(dashboardChartLocalized("field.date"), point.date),
                        y: .value(dashboardChartLocalized("field.balance"), Double(point.amountSats) / 100_000_000)
                    )
                    .foregroundStyle(.tint)
                }
                .chartXSelection(value: $selectedDate)
                .chartXAxis { AxisMarks(values: .automatic(desiredCount: 5)) }
                .chartYAxis { AxisMarks(position: .leading) }
                .chartLegend(.hidden)
                .frame(height: 150)
                .kassiberSensitive()
            }
            .padding(6)
        } label: {
            Label(dashboardChartLocalized("wallet.balanceHistory"), systemImage: "chart.xyaxis.line")
        }
    }

    private var changeLabel: String {
        guard let first = points.first, let last = points.last else { return "" }
        let delta = last.amountSats - first.amountSats
        let sign = delta >= 0 ? "+" : "−"
        return "\(sign)\(format(abs(delta)))"
    }

    private func nearest(to date: Date) -> WalletHistoryPoint? {
        points.min { abs($0.date.timeIntervalSince(date)) < abs($1.date.timeIntervalSince(date)) }
    }

    private func format(_ sats: Int64) -> String {
        let btc = Double(sats) / 100_000_000
        if currency == .euro, let priceEUR {
            return KassiberFormatting.fiat(btc * priceEUR, currency: "EUR", locale: .current)
        }
        return KassiberFormatting.btc(btc, locale: .current)
    }
}
