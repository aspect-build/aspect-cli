# Aspect Cli Config Structure

Status: **DRAFT**<br>
Date: _April 20, 2022_<br>
Authors: _Dylan Martin \<dylan@aspect.dev\>_<br>
Reviewers: _..._<br>
Summary: _How to structure the config files and the their layout for the aspect cli._<br>

---

**This document follows the conventions from [RFC 2119](https://www.ietf.org/rfc/rfc2119.txt).**

## Background

The design will consist of two sections:

1. The folder structure and the precedence of these folders in relation to one another.
2. The structure of the actual config file itself and how to interact with it.

There will be two configuration files that are found by default one in the base of the workspace and one in the users home directory. The user file that is located in the users home directory will take precedence. A user will also be able to use a single config file by specifying it's location through a aspect cli flag.

In the config file the plugins and features should use a namespace layout that can have nested maps. The goal here is to make it consistent and easy to understand what is enabled and what the expected behavior is of the aspect cli. Any value set in the config file should also be a value that can be passed through the aspect cli.

## Goals and non-goals

Goals:

After reading this document one should be able to answer all of these questions about the aspect cli.

Questions that should be answered in this proposal:

- what files are required for the cli?
- where are these files?
- what precedence do these file have?
- how do they interact with flags passed to the cli?
- which file is updated with changes?
- is there a separate command to modify config files?
- how does this work on ci?
- can I specify which file to use?
- does this create files in my home folder?
- do the cli and plugins all use the same config file?
- what is the structure of the config file?
- what is a plugin?
- what is a feature of a plugin?
- can I turn individual plugins off?
- can one plugin depend on another plugin?
- how do I specify if one plugin depends on another?
- can I turn individual features of a plugin off?
- what is a namespace for a plugin in the config file?
- can I nest namespaces for a plugin?
- can every option in the config file be passed to the cli?

The default behavior should be consistent with how bazel interacts with bazelrc files. As those using the aspect cli could be familiar with how bazel interacts with it's rc files already.

This document should provide enough information so that so that someone should be able to either understand how to setup a configuration file so that they can enable the plugins and features they would like to use.

By reading this document someone should be able to either understand how to operate a configuration file so that they can setup the plugins and features of those plugins that they would like to use. Or they should understand the configuration file that their plugin will need to consume and the format that it should use to be consistent with all other plugins.

Non-goals:

- how these choices should be implemented

Currently there is no agreed upon way for the config files to be laid out and for their precedence to be understood. The goal of one section of this document it to make it clear what configurations files are available, how they interact with one another, how they can be overridden and how they can be changed.

The goal of the second section of this document is to establish a clear format, expectation and language around the structure of the config file and its contents along with how that maps to plugins and features of those plugins.

## Proposal

First lets create some common definitions to remove ambiguity surrounding language with regards to the aspect cli.

## Definitions

### Plugin

For the aspect cli a plugin refers to something that exposes a grpc server this will typically be a binary.
The definition of a plugin is not associated with any functionality inside that binary or anything else but simply that it exposes a grpc server and interacts though that server.

Plugins can be turned on and off independently unless they are explicitly specified as a dependency of another plugin. This will be discussed further later on.

### Feature

A feature refers to some functionality that is exposed through a plugin. There can be multiple features in a single plugin.

Features should be exposed independently inside of a plugin so that they can be turned on and off independently.

## Config File Content Structure

The aspect cli config should be broken down into different namespaces that map to each plugin. This structure can be nested so that features of plugins can also use their own namespace structure to control their own features and settings. The core aspect cli will only deal with the top level namespaces and turing those plugins on appropriately. Dealing with the rest is up to the creator of the plugin. It is recommended that the same namespace structure is used to control features of a plugin as well so that it is easy and consistent for users to understand a config file with multiple plugins and update them as needed.

This should include a relevant namespace nested example with multiple plugins and features:

It is also relevant to know that all features available through the config file should also be flags that can be passed to the aspect cli. One should also consider that features and settings in the config file should be modifiable through a command in the aspect cli that writes to the config file. This is just to say take care in naming and structuring your namespace so that commands are readable on their own.

How to deal with plugin ordering and dependencies?
This is the lest discussed section. Open for comment. Should we just add a common top level way for aspect cli core to understand which plugins are required for others to run? How does this work with a plugin that has multiple features? Can the feature say it needs the plugin or does the top level have to say it requires that plugin? What does the error message and error handling look like here? How do we make sure there is an obvious and actionable error message for a user?

relevant github issues:
https://github.com/aspect-build/aspect-cli/issues/170

## Folder Structure

The workspace should have a .aspect folder at the base and then a config.yaml file inside which will contain the configuration for aspect cli.

$WORKSPACE/.aspect/config.yaml

The user should have their own .aspect folder in their $HOME directory and a config.yaml file inside which will contain the user settings that will take precedence.

$HOME/.apsect/config.yaml

The aspect cli will automatically look for configuration files in these locations. The user file ($HOME/.apsect/config.yaml) will take precedence over the workspace file ($WORKSPACE/.aspect/config.yaml). The interaction between the files should be similar to how bazel deals with .bazelrc files so it will be comfortable for those already familiar. Unless a flag is passed to the aspect cli with an exact location and in that case only that file will be used.

When a user wants to make a change the default file that will be changed should be made in user file. There should also be a command to update the config "aspect config {}" for example should be responsible to to actually update a file with a new desired config. This should be printed or prompted for the user to run.

relevant github issues:
https://github.com/aspect-build/aspect-cli/issues/75
