int count_down(int n) {
    int steps = 0;
    while (n > 0) {
        n -= 1;
        steps += 1;
    }
    return steps;
}

int less(int x) {
    int y = x < 3;
    return y;
}

int both(int x, int y) { return (x > 0) && (y > 0); }

int nested(int x) { return (x < 3) + 1; }
