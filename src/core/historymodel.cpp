#include "historymodel.h"
#include "daemonclient.h"

HistoryModel::HistoryModel(QObject *parent)
    : QAbstractListModel(parent)
{
    if (auto *d = DaemonClient::instance())
        connect(d, &DaemonClient::reconnected, this, &HistoryModel::reload);
}

int HistoryModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : int(m_rows.size());
}

QVariant HistoryModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.row() >= m_rows.size())
        return {};
    const QVariantMap m = m_rows.at(index.row()).toMap();
    switch (role) {
    case IdRole:
        return m.value(QStringLiteral("id"));
    case TimeRole:
        return m.value(QStringLiteral("time"));
    case SaidRole:
        return m.value(QStringLiteral("said"));
    case DidRole:
        return m.value(QStringLiteral("did"));
    case ResultRole:
        return m.value(QStringLiteral("result"));
    case LayerRole:
        return m.value(QStringLiteral("layer"));
    case DurationRole:
        return m.value(QStringLiteral("durationMs"));
    case AppRole:
        return m.value(QStringLiteral("app"));
    case ActionRole:
        return m.value(QStringLiteral("action"));
    case UndoableRole:
        return m.value(QStringLiteral("undoable"));
    case ItemRole:
        return m;
    default:
        return {};
    }
}

QHash<int, QByteArray> HistoryModel::roleNames() const
{
    return {{IdRole, "itemId"}, {TimeRole, "time"}, {SaidRole, "said"}, {DidRole, "did"}, {ResultRole, "result"}, {LayerRole, "layer"},
            {DurationRole, "durationMs"}, {AppRole, "app"}, {ActionRole, "action"}, {UndoableRole, "undoable"}, {ItemRole, "item"}};
}

bool HistoryModel::canFetchMore(const QModelIndex &parent) const
{
    return !parent.isValid() && hasMore() && !m_loading;
}

void HistoryModel::fetchMore(const QModelIndex &parent)
{
    if (canFetchMore(parent))
        fetch(false);
}

void HistoryModel::setFilters(const QVariantMap &f)
{
    if (f == m_filters)
        return;
    m_filters = f;
    emit filtersChanged();
    reload();
}

void HistoryModel::setSortKey(const QString &k)
{
    if (k == m_sortKey)
        return;
    m_sortKey = k;
    emit sortChanged();
    reload();
}

void HistoryModel::setSortDesc(bool d)
{
    if (d == m_desc)
        return;
    m_desc = d;
    emit sortChanged();
    reload();
}

void HistoryModel::setPageSize(int n)
{
    n = qBound(1, n, 200);
    if (n == m_page)
        return;
    m_page = n;
    emit pageSizeChanged();
}

void HistoryModel::reload()
{
    fetch(true);
}

void HistoryModel::loadMore()
{
    if (hasMore() && !m_loading)
        fetch(false);
}

void HistoryModel::fetch(bool reset)
{
    auto *d = DaemonClient::instance();
    if (!d)
        return;
    const quint64 gen = reset ? ++m_generation : m_generation;
    QVariantMap filters;
    for (auto it = m_filters.cbegin(); it != m_filters.cend(); ++it)
        if (it.value().isValid() && !it.value().toString().isEmpty())
            filters.insert(it.key(), it.value());
    QVariantMap p{{QStringLiteral("limit"), m_page}, {QStringLiteral("filters"), filters},
                  {QStringLiteral("sort"), QVariantMap{{QStringLiteral("key"), m_sortKey}, {QStringLiteral("desc"), m_desc}}}};
    if (!reset && !m_cursor.isEmpty())
        p.insert(QStringLiteral("cursor"), m_cursor);
    m_loading = true;
    m_error.clear();
    emit stateChanged();
    d->request(QStringLiteral("history.query"), p, [this, gen, reset](bool ok, const QVariant &result, const QVariantMap &error) {
        if (gen != m_generation)
            return;
        m_loading = false;
        if (!ok) {
            m_error = error.value(QStringLiteral("message")).toString();
            emit stateChanged();
            return;
        }
        const QVariantMap r = result.toMap();
        const QVariantList items = r.value(QStringLiteral("items")).toList();
        m_cursor = r.value(QStringLiteral("nextCursor")).toString();
        m_total = r.value(QStringLiteral("total")).toInt();
        if (reset) {
            beginResetModel();
            m_rows = items;
            endResetModel();
        } else if (!items.isEmpty()) {
            beginInsertRows({}, int(m_rows.size()), int(m_rows.size() + items.size() - 1));
            m_rows.append(items);
            endInsertRows();
        }
        emit stateChanged();
    });
}

QVariantMap HistoryModel::get(int row) const
{
    return row >= 0 && row < m_rows.size() ? m_rows.at(row).toMap() : QVariantMap();
}

void HistoryModel::prepend(const QVariantMap &item)
{
    if (m_sortKey != QLatin1String("time") || !m_desc || !m_filters.isEmpty())
        return;
    beginInsertRows({}, 0, 0);
    m_rows.prepend(item);
    endInsertRows();
    ++m_total;
    emit stateChanged();
}

void HistoryModel::removeIds(const QStringList &ids)
{
    for (int i = int(m_rows.size()) - 1; i >= 0; --i) {
        if (ids.contains(m_rows.at(i).toMap().value(QStringLiteral("id")).toString())) {
            beginRemoveRows({}, i, i);
            m_rows.removeAt(i);
            endRemoveRows();
            --m_total;
        }
    }
    emit stateChanged();
}
