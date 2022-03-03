/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package logger

import (
	"fmt"
	"io"
	"os"

	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/go-homedir"
	"github.com/natefinch/lumberjack"
)

const (
	defaultLevel = hclog.Trace
)

type GlobalLoggerStruct struct {
	aspect  hclog.Logger
	command hclog.Logger
}

var GlobalLogger GlobalLoggerStruct

type Logger interface {
	Trace(msg string, args ...interface{})
	Debug(msg string, args ...interface{})
	Info(msg string, args ...interface{})
	Warn(msg string, args ...interface{})
	Error(msg string, args ...interface{})
}

func CreateLogger(name string) Logger {
	return GlobalLogger.aspect.Named(name)
}

func getHomeDir() string {
	home, err := homedir.Dir()
	if err != nil {
		fmt.Fprintln(os.Stderr, "Error:", err)
		os.Exit(1)
	}

	return home
}

func getPrefix(invocationId string) string {
	prefix := getHomeDir() + "/.aspectlogs/" + invocationId
	os.MkdirAll(prefix, 0700)
	return prefix
}

func generateLumberjack(filename string) *lumberjack.Logger {
	return &lumberjack.Logger{
		Filename:   filename,
		MaxSize:    1,
		MaxBackups: 3,
		MaxAge:     28,
	}
}

func CreateGlobalLogger(invocationId string) {
	prefix := getPrefix(invocationId)

	GlobalLogger = GlobalLoggerStruct{}

	GlobalLogger.aspect = hclog.New(&hclog.LoggerOptions{
		Level:  defaultLevel,
		Output: generateLumberjack(prefix + "/aspect.log"),
	})

	GlobalLogger.command = hclog.New(&hclog.LoggerOptions{
		Level:  defaultLevel,
		Output: generateLumberjack(prefix + "/command.log"),
	})

}

func CreateMockLogger(w io.Writer) {
	l := hclog.New(&hclog.LoggerOptions{
		Level:  defaultLevel,
		Output: w,
	})

	GlobalLogger = GlobalLoggerStruct{}

	GlobalLogger.aspect = l
	GlobalLogger.command = l

}

func CreatePluginLogger() Logger {
	GlobalLogger = GlobalLoggerStruct{}

	l := hclog.New(&hclog.LoggerOptions{
		Level:      defaultLevel,
		Output:     os.Stderr,
		JSONFormat: true,
	})

	GlobalLogger.aspect = l
	GlobalLogger.command = l

	return l
}

func logLevel(level string) hclog.Level {
	logLevel := hclog.LevelFromString(level)
	if logLevel == hclog.NoLevel {
		logLevel = hclog.Error
	}

	return logLevel
}

func CreatePluginLoggingHook(invocationId string, pluginName string, level string) hclog.Logger {
	logLevel := logLevel(level)

	pluginLogger := hclog.New(&hclog.LoggerOptions{
		Name:   pluginName,
		Level:  logLevel,
		Output: generateLumberjack(getPrefix(invocationId) + "/plugin-" + pluginName + ".log"),
	})

	return pluginLogger
}

func Trace(message string, args ...interface{}) {
	GlobalLogger.aspect.Trace(message, args...)
}

func Debug(message string, args ...interface{}) {
	GlobalLogger.aspect.Debug(message, args...)
}

func Info(message string, args ...interface{}) {
	GlobalLogger.aspect.Info(message, args...)
}

func Warn(message string, args ...interface{}) {
	GlobalLogger.aspect.Warn(message, args...)
}

func Error(message string, args ...interface{}) {
	GlobalLogger.aspect.Error(message, args...)
}

func Command(message string) {
	GlobalLogger.command.Info(message)
}
