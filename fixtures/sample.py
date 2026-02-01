import os
import sys  # unused import

def main():
    result = process_data("hello")
    print(result)

def process_data(data):
    return data.upper()

def unused_function():
    """This function is never called."""
    return 42

def another_dead_code():
    """Also never called."""
    password = "super_secret_password_123"
    return password

class UnusedClass:
    def method(self):
        pass
