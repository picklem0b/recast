enum color { RED, GREEN = 5, BLUE };

int color_index(enum color c) {
    switch (c) {
    case RED:
        return 0;
    case GREEN:
        return 1;
    case BLUE:
        return 2;
    default:
        return -1;
    }
}

int is_warm(enum color c) {
    return c == GREEN || c == RED;
}

enum color next_color(enum color c) {
    if (c == RED) {
        return GREEN;
    }
    if (c == GREEN) {
        return BLUE;
    }
    return RED;
}
