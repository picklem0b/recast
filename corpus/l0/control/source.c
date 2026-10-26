int classify(int n) {
    switch (n) {
    case 0:
        return 0;
    case 1:
    case 2:
        return 1;
    default:
        return 2;
    }
}

int sign(int n) {
    if (n < 0) {
        return -1;
    } else if (n == 0) {
        return 0;
    }
    return 1;
}

int clamp(int value, int low, int high) {
    if (value < low) {
        return low;
    }
    if (value > high) {
        return high;
    }
    return value;
}

int count_down(int n) {
    int total = 0;
    while (n > 0) {
        if (n == 3) {
            n -= 1;
            continue;
        }
        if (n == 1) {
            break;
        }
        total += n;
        n -= 1;
    }
    return total;
}

int do_sum(int n) {
    int total = 0;
    int i = 0;
    do {
        total += i;
        i++;
    } while (i <= n);
    return total;
}

int label_value(int n) {
    return n > 10 ? 100 : n * 2;
}
