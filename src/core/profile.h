#pragma once

#include <QString>

struct Profile
{
    QString name;
    QString login;
    QString avatar;
};

namespace profile {
Profile read();
}
