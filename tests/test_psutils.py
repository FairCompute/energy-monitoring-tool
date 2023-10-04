import psutil

if __name__ == '__main__':
    p = psutil.Process()
    print(psutil.cpu_percent(100, percpu=False))
    print(f'{p.cmdline()}: {p.cpu_percent(interval=1)}')
          