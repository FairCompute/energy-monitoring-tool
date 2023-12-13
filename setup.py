from setuptools import setup

setup(name='emt',
    python_requires='>3.9',
    version='0.1.0',
    description='',
    author='Rameez Ismail',
    author_email='rameez.ismail@philips.com',
    install_requires=[
        'psutil',
        'numpy',
        'pandas',
        'pynvml',
        'tensorflow[and-cuda]',
    ],
)
